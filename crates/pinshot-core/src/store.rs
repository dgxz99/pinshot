use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use image::imageops::FilterType;
use image::{GenericImageView, ImageFormat};
use serde::{Deserialize, Serialize};

use crate::config::CacheConfig;
use crate::error::{PinshotError, Result};
use crate::protocol::{HistoryItemSummary, PinKind, PinPayload, Point};

/// 缩略图最大边长（像素）
const THUMB_MAX_EDGE: u32 = 128;

/// 单进程内 ID 生成自增序号，避免同一毫秒内碰撞
static ID_SEQ: AtomicU64 = AtomicU64::new(0);

/// 持久化索引结构（`index.json`）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoreIndex {
    version: u32,
    entries: Vec<PinRecord>,
}

/// 单条贴图与历史缓存记录
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PinRecord {
    pub id: String,
    pub kind: PinKind,
    pub created_at: u64,
    pub last_used_at: u64,
    /// 上次是否处于显示态
    pub active: bool,
    pub image_path: PathBuf,
    pub thumb_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_path: Option<PathBuf>,
    pub title_preview: String,
    pub width: u32,
    pub height: u32,
    pub source_scale: f64,
    pub size_bytes: u64,
}

impl PinRecord {
    /// 转换为发送给前端的 D-Bus `PinPayload`
    pub fn to_payload(&self, initial_pos: Option<Point>) -> PinPayload {
        PinPayload {
            id: self.id.clone(),
            image_path: self.image_path.to_string_lossy().into_owned(),
            thumb_path: self.thumb_path.to_string_lossy().into_owned(),
            width: self.width,
            height: self.height,
            source_scale: self.source_scale,
            kind: self.kind,
            has_text: self.text_path.is_some(),
            title_preview: self.title_preview.clone(),
            initial_pos,
        }
    }

    /// 转换为返回给顶栏历史列表的轻量摘要
    pub fn to_summary(&self) -> HistoryItemSummary {
        HistoryItemSummary {
            id: self.id.clone(),
            kind: self.kind,
            created_at: self.created_at,
            last_used_at: self.last_used_at,
            active: self.active,
            thumb_path: self.thumb_path.to_string_lossy().into_owned(),
            has_text: self.text_path.is_some(),
            title_preview: self.title_preview.clone(),
            width: self.width,
            height: self.height,
            size_bytes: self.size_bytes,
        }
    }
}

/// 新增缓存条目时的输入参数
pub struct NewEntryRequest<'a> {
    pub kind: PinKind,
    pub png_bytes: &'a [u8],
    pub text_content: Option<&'a str>,
    pub title_preview: String,
    pub source_scale: f64,
    pub active: bool,
}

/// 缓存清理统计结果
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneStats {
    pub removed_count: usize,
    pub freed_bytes: u64,
}

/// 持久化缓存管理器（对应 `~/.cache/pinshot/`）
#[derive(Debug, Clone)]
pub struct PinStore {
    root_dir: PathBuf,
}

impl PinStore {
    /// 使用默认 XDG 缓存目录初始化（`~/.cache/pinshot/`）
    pub fn open_default() -> Result<Self> {
        let base = dirs::cache_dir()
            .ok_or_else(|| PinshotError::HomeDir("无法获取 XDG cache 目录".to_string()))?;
        Self::open_at(base.join("pinshot"))
    }

    /// 在指定根目录初始化缓存结构并确保权限为 `0700`
    pub fn open_at(root_dir: PathBuf) -> Result<Self> {
        let store = Self { root_dir };
        store.ensure_layout()?;
        Ok(store)
    }

    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    pub fn images_dir(&self) -> PathBuf {
        self.root_dir.join("images")
    }

    pub fn thumbs_dir(&self) -> PathBuf {
        self.root_dir.join("thumbs")
    }

    pub fn texts_dir(&self) -> PathBuf {
        self.root_dir.join("texts")
    }

    pub fn index_path(&self) -> PathBuf {
        self.root_dir.join("index.json")
    }

    pub fn screencast_token_path(&self) -> PathBuf {
        self.root_dir.join("screencast_token")
    }

    /// 创建缓存目录树并严格限制权限为 `0700`
    fn ensure_layout(&self) -> Result<()> {
        for dir in [
            self.root_dir.clone(),
            self.images_dir(),
            self.thumbs_dir(),
            self.texts_dir(),
        ] {
            if !dir.exists() {
                fs::create_dir_all(&dir).map_err(|e| PinshotError::io(&dir, e))?;
            }
            set_unix_mode(&dir, 0o700)?;
        }
        Ok(())
    }

    /// 写入新 PNG 条目、生成缩略图、更新索引并触发双阈值淘汰
    pub fn add_png_entry(
        &self,
        req: NewEntryRequest<'_>,
        cache_cfg: &CacheConfig,
    ) -> Result<PinRecord> {
        let now = unix_now_secs();
        self.add_png_entry_at(req, cache_cfg, now)
    }

    /// 指定时间戳的新增条目实现（便于测试双阈值淘汰逻辑）
    pub fn add_png_entry_at(
        &self,
        req: NewEntryRequest<'_>,
        cache_cfg: &CacheConfig,
        now: u64,
    ) -> Result<PinRecord> {
        self.ensure_layout()?;

        let img = image::load_from_memory(req.png_bytes)?;
        let (width, height) = img.dimensions();
        let id = generate_pin_id();

        let image_path = self.images_dir().join(format!("{id}.png"));
        let thumb_path = self.thumbs_dir().join(format!("{id}.png"));
        let text_path = req
            .text_content
            .map(|_| self.texts_dir().join(format!("{id}.txt")));

        write_file_private(&image_path, req.png_bytes)?;
        let thumb_bytes = render_thumbnail_png(&img, THUMB_MAX_EDGE)?;
        write_file_private(&thumb_path, &thumb_bytes)?;

        let mut total_size = req.png_bytes.len() as u64 + thumb_bytes.len() as u64;
        if let (Some(path), Some(text)) = (&text_path, req.text_content) {
            write_file_private(path, text.as_bytes())?;
            total_size += text.len() as u64;
        }

        let source_scale = if req.source_scale > 0.0 {
            req.source_scale
        } else {
            1.0
        };

        let record = PinRecord {
            id,
            kind: req.kind,
            created_at: now,
            last_used_at: now,
            active: req.active,
            image_path,
            thumb_path,
            text_path,
            title_preview: req.title_preview,
            width,
            height,
            source_scale,
            size_bytes: total_size,
        };

        let mut index = self.load_index()?;
        index.entries.push(record.clone());
        let max_bytes = cache_cfg.max_size_mb.saturating_mul(1024 * 1024);
        let max_age_secs = cache_cfg.max_age_days.saturating_mul(86_400);
        prune_index_entries(&mut index.entries, now, max_bytes, max_age_secs);
        self.save_index(&index)?;

        Ok(record)
    }

    /// 标注回流替换图片：将旧版本另存为一条非活跃历史记录防后悔，同时原地更新当前贴图文件与尺寸
    pub fn replace_entry_image(
        &self,
        id: &str,
        new_png_bytes: &[u8],
        cache_cfg: &CacheConfig,
    ) -> Result<PinRecord> {
        self.ensure_layout()?;
        let now = unix_now_secs();
        let img = image::load_from_memory(new_png_bytes)?;
        let (width, height) = img.dimensions();
        let thumb_bytes = render_thumbnail_png(&img, THUMB_MAX_EDGE)?;

        let mut index = self.load_index()?;
        let pos = index
            .entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| PinshotError::NotFound(id.to_string()))?;

        // 将旧版本保留为一条非活跃历史，防止用户后悔
        let old_entry = index.entries[pos].clone();
        if old_entry.image_path.exists() {
            let backup_id = generate_pin_id();
            let backup_img_path = self.images_dir().join(format!("{backup_id}.png"));
            let backup_thumb_path = self.thumbs_dir().join(format!("{backup_id}.png"));
            fs::copy(&old_entry.image_path, &backup_img_path)
                .map_err(|e| PinshotError::io(&backup_img_path, e))?;
            set_unix_mode(&backup_img_path, 0o600)?;

            let mut backup_size = fs::metadata(&backup_img_path)
                .map_err(|e| PinshotError::io(&backup_img_path, e))?
                .len();

            if old_entry.thumb_path.exists() {
                fs::copy(&old_entry.thumb_path, &backup_thumb_path)
                    .map_err(|e| PinshotError::io(&backup_thumb_path, e))?;
                set_unix_mode(&backup_thumb_path, 0o600)?;
                backup_size += fs::metadata(&backup_thumb_path)
                    .map_err(|e| PinshotError::io(&backup_thumb_path, e))?
                    .len();
            }

            let backup_record = PinRecord {
                id: backup_id,
                kind: old_entry.kind,
                created_at: old_entry.created_at,
                last_used_at: now.saturating_sub(1),
                active: false,
                image_path: backup_img_path,
                thumb_path: backup_thumb_path,
                text_path: None,
                title_preview: format!("{}（标注前）", old_entry.title_preview),
                width: old_entry.width,
                height: old_entry.height,
                source_scale: old_entry.source_scale,
                size_bytes: backup_size,
            };
            index.entries.push(backup_record);
        }

        // 用新图片覆盖当前贴图绑定的文件
        let target = &mut index.entries[pos];
        write_file_private(&target.image_path, new_png_bytes)?;
        write_file_private(&target.thumb_path, &thumb_bytes)?;

        let text_bytes = target
            .text_path
            .as_ref()
            .and_then(|p| fs::metadata(p).ok())
            .map(|m| m.len())
            .unwrap_or(0);

        target.width = width;
        target.height = height;
        target.last_used_at = now;
        target.active = true;
        target.size_bytes = new_png_bytes.len() as u64 + thumb_bytes.len() as u64 + text_bytes;

        let updated = target.clone();
        let max_bytes = cache_cfg.max_size_mb.saturating_mul(1024 * 1024);
        let max_age_secs = cache_cfg.max_age_days.saturating_mul(86_400);
        prune_index_entries(&mut index.entries, now, max_bytes, max_age_secs);
        self.save_index(&index)?;

        Ok(updated)
    }

    /// 按 ID 查询缓存记录
    pub fn get(&self, id: &str) -> Result<Option<PinRecord>> {
        let index = self.load_index()?;
        Ok(index.entries.into_iter().find(|entry| entry.id == id))
    }

    /// 标记指定贴图为已关闭（保留在历史中以便捞回）
    pub fn mark_closed(&self, id: &str) -> Result<bool> {
        self.set_active_state(id, false)
    }

    /// 标记指定贴图为活跃状态并刷新最近使用时间
    pub fn mark_active(&self, id: &str) -> Result<bool> {
        self.set_active_state(id, true)
    }

    fn set_active_state(&self, id: &str, active: bool) -> Result<bool> {
        let mut index = self.load_index()?;
        let now = unix_now_secs();
        let mut changed = false;
        for entry in &mut index.entries {
            if entry.id == id {
                entry.active = active;
                entry.last_used_at = now;
                changed = true;
                break;
            }
        }
        if changed {
            self.save_index(&index)?;
        }
        Ok(changed)
    }

    /// 获取所有当前标记为 `active = true` 的记录（按 `last_used_at` 升序，用于启动恢复）
    pub fn list_active(&self) -> Result<Vec<PinRecord>> {
        let index = self.load_index()?;
        let mut items: Vec<PinRecord> = index
            .entries
            .into_iter()
            .filter(|entry| entry.active && entry.image_path.exists())
            .collect();
        items.sort_by_key(|entry| (entry.last_used_at, entry.created_at));
        Ok(items)
    }

    /// 按最近使用时间倒序列出历史条目（优先过滤磁盘文件已丢失的无效项）
    pub fn list_history(&self, limit: usize) -> Result<Vec<PinRecord>> {
        let index = self.load_index()?;
        let mut items: Vec<PinRecord> = index
            .entries
            .into_iter()
            .filter(|entry| entry.image_path.exists())
            .collect();
        items.sort_by(|a, b| {
            b.last_used_at
                .cmp(&a.last_used_at)
                .then_with(|| b.created_at.cmp(&a.created_at))
        });
        items.truncate(limit);
        Ok(items)
    }

    /// 查找最近一条可恢复的记录（排除前端当前正在显示的活跃贴图 ID 集合）
    pub fn latest_restorable(&self, currently_visible_ids: &HashSet<String>) -> Result<Option<PinRecord>> {
        let index = self.load_index()?;
        let mut candidates: Vec<PinRecord> = index
            .entries
            .into_iter()
            .filter(|entry| {
                !currently_visible_ids.contains(&entry.id) && entry.image_path.exists()
            })
            .collect();

        // 重启后遗留 active=true 的历史记录排在最前，其次按 last_used_at 倒序
        candidates.sort_by(|a, b| {
            b.active
                .cmp(&a.active)
                .then_with(|| b.last_used_at.cmp(&a.last_used_at))
                .then_with(|| b.created_at.cmp(&a.created_at))
        });

        Ok(candidates.into_iter().next())
    }

    /// 彻底物理删除单条历史的图片、缩略图、文本与索引
    pub fn delete_entry(&self, id: &str) -> Result<bool> {
        let mut index = self.load_index()?;
        let Some(pos) = index.entries.iter().position(|entry| entry.id == id) else {
            return Ok(false);
        };
        let removed = index.entries.remove(pos);
        remove_record_files(&removed);
        self.save_index(&index)?;
        Ok(true)
    }

    /// 清空历史缓存：默认仅清非活跃条目；`include_active = true` 时全部清空并返回被删活跃 ID
    pub fn clear_history(&self, include_active: bool) -> Result<(usize, Vec<String>)> {
        let mut index = self.load_index()?;
        let mut removed_count = 0usize;
        let mut removed_active_ids = Vec::new();
        let mut kept = Vec::with_capacity(index.entries.len());

        for entry in index.entries {
            if include_active || !entry.active {
                if entry.active {
                    removed_active_ids.push(entry.id.clone());
                }
                remove_record_files(&entry);
                removed_count += 1;
            } else {
                kept.push(entry);
            }
        }

        index.entries = kept;
        self.save_index(&index)?;
        Ok((removed_count, removed_active_ids))
    }

    /// 显式执行双阈值清理（支持指定时间戳与字节/秒阈值，便于单测与运维调用）
    pub fn enforce_retention_at(
        &self,
        now: u64,
        max_bytes: u64,
        max_age_secs: u64,
    ) -> Result<PruneStats> {
        let mut index = self.load_index()?;
        let stats = prune_index_entries(&mut index.entries, now, max_bytes, max_age_secs);
        if stats.removed_count > 0 {
            self.save_index(&index)?;
        }
        Ok(stats)
    }

    /// 读取持久化的 ScreenCast restore_token
    pub fn load_screencast_token(&self) -> Result<Option<String>> {
        let path = self.screencast_token_path();
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path).map_err(|e| PinshotError::io(&path, e))?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            Ok(None)
        } else {
            Ok(Some(trimmed.to_string()))
        }
    }

    /// 以 `0600` 权限持久化 ScreenCast restore_token
    pub fn save_screencast_token(&self, token: &str) -> Result<()> {
        self.ensure_layout()?;
        let path = self.screencast_token_path();
        write_file_private(&path, token.trim().as_bytes())
    }

    /// 清除失效的 ScreenCast restore_token
    pub fn clear_screencast_token(&self) -> Result<()> {
        let path = self.screencast_token_path();
        if path.exists() {
            fs::remove_file(&path).map_err(|e| PinshotError::io(&path, e))?;
        }
        Ok(())
    }

    fn load_index(&self) -> Result<StoreIndex> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(StoreIndex {
                version: 1,
                entries: Vec::new(),
            });
        }
        let content = fs::read_to_string(&path).map_err(|e| PinshotError::io(&path, e))?;
        if content.trim().is_empty() {
            return Ok(StoreIndex {
                version: 1,
                entries: Vec::new(),
            });
        }
        let index: StoreIndex = serde_json::from_str(&content)?;
        Ok(index)
    }

    /// 原子写入 `index.json`：先写同目录临时文件并刷新落盘，再 `rename` 替换目标
    fn save_index(&self, index: &StoreIndex) -> Result<()> {
        self.ensure_layout()?;
        let target = self.index_path();
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp_path = self
            .root_dir
            .join(format!(".index.json.tmp.{pid}.{nanos}"));

        let serialized = serde_json::to_vec_pretty(index)?;
        {
            let mut file =
                fs::File::create(&tmp_path).map_err(|e| PinshotError::io(&tmp_path, e))?;
            file.write_all(&serialized)
                .map_err(|e| PinshotError::io(&tmp_path, e))?;
            file.sync_all()
                .map_err(|e| PinshotError::io(&tmp_path, e))?;
        }
        set_unix_mode(&tmp_path, 0o600)?;
        fs::rename(&tmp_path, &target).map_err(|e| PinshotError::io(&target, e))?;
        Ok(())
    }
}

/// 执行双阈值自动淘汰：保护活跃贴图，先按过期天数清理，再按总容量淘汰最旧非活跃条目
fn prune_index_entries(
    entries: &mut Vec<PinRecord>,
    now: u64,
    max_bytes: u64,
    max_age_secs: u64,
) -> PruneStats {
    let mut stats = PruneStats::default();

    // 1. 先淘汰超过保留天数的非活跃条目
    let mut remaining = Vec::with_capacity(entries.len());
    for entry in entries.drain(..) {
        let expired = !entry.active && now.saturating_sub(entry.last_used_at) > max_age_secs;
        if expired {
            remove_record_files(&entry);
            stats.removed_count += 1;
            stats.freed_bytes = stats.freed_bytes.saturating_add(entry.size_bytes);
        } else {
            remaining.push(entry);
        }
    }

    // 2. 再检查总占用容量，超限时按 last_used_at 从旧到新淘汰非活跃条目
    let mut total_bytes: u64 = remaining.iter().map(|e| e.size_bytes).sum();
    if total_bytes > max_bytes {
        let mut evictable_indices: Vec<usize> = remaining
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.active)
            .map(|(idx, _)| idx)
            .collect();

        evictable_indices.sort_by_key(|&idx| {
            (remaining[idx].last_used_at, remaining[idx].created_at)
        });

        let mut to_remove = HashSet::new();
        for idx in evictable_indices {
            if total_bytes <= max_bytes {
                break;
            }
            let entry = &remaining[idx];
            remove_record_files(entry);
            total_bytes = total_bytes.saturating_sub(entry.size_bytes);
            stats.removed_count += 1;
            stats.freed_bytes = stats.freed_bytes.saturating_add(entry.size_bytes);
            to_remove.insert(idx);
        }

        if !to_remove.is_empty() {
            let mut filtered = Vec::with_capacity(remaining.len() - to_remove.len());
            for (idx, entry) in remaining.into_iter().enumerate() {
                if !to_remove.contains(&idx) {
                    filtered.push(entry);
                }
            }
            remaining = filtered;
        }
    }

    *entries = remaining;
    stats
}

/// 删除单条记录关联的磁盘文件（忽略已不存在的文件）
fn remove_record_files(record: &PinRecord) {
    let _ = remove_if_exists(&record.image_path);
    let _ = remove_if_exists(&record.thumb_path);
    if let Some(text_path) = &record.text_path {
        let _ = remove_if_exists(text_path);
    }
}

fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// 生成高质量轻量 PNG 缩略图（限制在 `max_edge × max_edge` 内）
fn render_thumbnail_png(img: &image::DynamicImage, max_edge: u32) -> Result<Vec<u8>> {
    let (w, h) = img.dimensions();
    let resized = if w > max_edge || h > max_edge {
        img.resize(max_edge, max_edge, FilterType::Triangle)
    } else {
        img.clone()
    };

    let mut cursor = std::io::Cursor::new(Vec::new());
    resized.write_to(&mut cursor, ImageFormat::Png)?;
    Ok(cursor.into_inner())
}

fn write_file_private(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).map_err(|e| PinshotError::io(path, e))?;
    set_unix_mode(path, 0o600)?;
    Ok(())
}

fn set_unix_mode(path: &Path, mode: u32) -> Result<()> {
    let perms = fs::Permissions::from_mode(mode);
    fs::set_permissions(path, perms).map_err(|e| PinshotError::io(path, e))?;
    Ok(())
}

pub fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn generate_pin_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let pid = std::process::id() & 0xFFFF;
    let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed) & 0xFFFF;
    format!("{millis:x}-{pid:04x}-{seq:04x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};
    use tempfile::TempDir;

    fn make_test_png(width: u32, height: u32, seed: u8) -> Vec<u8> {
        let img = ImageBuffer::from_fn(width, height, |x, y| {
            let r = ((x as u8).wrapping_mul(3)).wrapping_add(seed);
            let g = ((y as u8).wrapping_mul(5)).wrapping_add(seed);
            let b = ((x ^ y) as u8).wrapping_add(seed);
            Rgba([r, g, b, 255])
        });
        let dyn_img = image::DynamicImage::ImageRgba8(img);
        let mut buf = std::io::Cursor::new(Vec::new());
        dyn_img
            .write_to(&mut buf, ImageFormat::Png)
            .expect("编码测试 PNG 失败");
        buf.into_inner()
    }

    #[test]
    fn store_creates_0700_dirs_and_persists_entry_with_thumbnail_and_text() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let store = PinStore::open_at(dir.path().join("pinshot")).expect("打开缓存失败");

        let root_mode = fs::metadata(store.root_dir())
            .expect("读取目录元数据失败")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(root_mode, 0o700, "缓存根目录权限应为 0700");

        let png = make_test_png(300, 150, 7);
        let rec = store
            .add_png_entry(
                NewEntryRequest {
                    kind: PinKind::ClipboardText,
                    png_bytes: &png,
                    text_content: Some("hello pinshot\n第二行"),
                    title_preview: "hello pinshot".to_string(),
                    source_scale: 1.5,
                    active: true,
                },
                &CacheConfig::default(),
            )
            .expect("写入缓存条目失败");

        assert_eq!(rec.width, 300);
        assert_eq!(rec.height, 150);
        assert!(rec.image_path.exists());
        assert!(rec.thumb_path.exists());
        let text_path = rec.text_path.as_ref().expect("应存在原文文件");
        assert_eq!(
            fs::read_to_string(text_path).expect("读取原文失败"),
            "hello pinshot\n第二行"
        );

        // 验证缩略图等比缩放到 128px 以内
        let thumb_img = image::open(&rec.thumb_path).expect("读取缩略图失败");
        assert_eq!(thumb_img.dimensions(), (128, 64));

        // 验证文件权限 0600
        let img_mode = fs::metadata(&rec.image_path)
            .expect("读取图片权限失败")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(img_mode, 0o600);

        // 重新查询索引验证原子落盘
        let loaded = store
            .get(&rec.id)
            .expect("查询索引失败")
            .expect("应找到已写入记录");
        assert_eq!(loaded, rec);
    }

    #[test]
    fn retention_evicts_expired_inactive_but_protects_active() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let store = PinStore::open_at(dir.path().join("pinshot")).expect("打开缓存失败");
        let large_cfg = CacheConfig {
            max_size_mb: 200,
            max_age_days: 14,
            history_menu_limit: 15,
        };

        let png = make_test_png(64, 64, 1);
        let day = 86_400u64;
        let base_t = 1_700_000_000u64;

        let expired_inactive = store
            .add_png_entry_at(
                NewEntryRequest {
                    kind: PinKind::Screenshot,
                    png_bytes: &png,
                    text_content: None,
                    title_preview: "过期非活跃".to_string(),
                    source_scale: 1.0,
                    active: false,
                },
                &large_cfg,
                base_t,
            )
            .expect("写入失败");

        let expired_active = store
            .add_png_entry_at(
                NewEntryRequest {
                    kind: PinKind::Screenshot,
                    png_bytes: &png,
                    text_content: None,
                    title_preview: "过期但活跃".to_string(),
                    source_scale: 1.0,
                    active: true,
                },
                &large_cfg,
                base_t,
            )
            .expect("写入失败");

        let fresh_inactive = store
            .add_png_entry_at(
                NewEntryRequest {
                    kind: PinKind::Screenshot,
                    png_bytes: &png,
                    text_content: None,
                    title_preview: "近期非活跃".to_string(),
                    source_scale: 1.0,
                    active: false,
                },
                &large_cfg,
                base_t + 10 * day,
            )
            .expect("写入失败");

        // 第 15 天触发清理：base_t 距今 15 天 > 14 天
        let stats = store
            .enforce_retention_at(base_t + 15 * day, 200 * 1024 * 1024, 14 * day)
            .expect("清理失败");

        assert_eq!(stats.removed_count, 1);
        assert!(store.get(&expired_inactive.id).unwrap().is_none());
        assert!(!expired_inactive.image_path.exists());
        assert!(store.get(&expired_active.id).unwrap().is_some(), "活跃贴图必须被豁免");
        assert!(store.get(&fresh_inactive.id).unwrap().is_some());
    }

    #[test]
    fn retention_evicts_oldest_inactive_when_exceeding_max_size() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let store = PinStore::open_at(dir.path().join("pinshot")).expect("打开缓存失败");
        let cfg = CacheConfig {
            max_size_mb: 200,
            max_age_days: 14,
            history_menu_limit: 15,
        };

        let png_a = make_test_png(120, 120, 10);
        let png_b = make_test_png(120, 120, 20);
        let png_c = make_test_png(120, 120, 30);
        let t0 = 1_700_000_000u64;

        let oldest_inactive = store
            .add_png_entry_at(
                NewEntryRequest {
                    kind: PinKind::Screenshot,
                    png_bytes: &png_a,
                    text_content: None,
                    title_preview: "最旧".to_string(),
                    source_scale: 1.0,
                    active: false,
                },
                &cfg,
                t0 + 10,
            )
            .unwrap();

        let newer_inactive = store
            .add_png_entry_at(
                NewEntryRequest {
                    kind: PinKind::Screenshot,
                    png_bytes: &png_b,
                    text_content: None,
                    title_preview: "较新".to_string(),
                    source_scale: 1.0,
                    active: false,
                },
                &cfg,
                t0 + 20,
            )
            .unwrap();

        let active_large = store
            .add_png_entry_at(
                NewEntryRequest {
                    kind: PinKind::Screenshot,
                    png_bytes: &png_c,
                    text_content: None,
                    title_preview: "活跃受保护".to_string(),
                    source_scale: 1.0,
                    active: true,
                },
                &cfg,
                t0 + 5,
            )
            .unwrap();

        // 限制容量仅够容纳 active_large + newer_inactive，放不下 oldest_inactive
        let budget = active_large.size_bytes + newer_inactive.size_bytes;
        let stats = store
            .enforce_retention_at(t0 + 30, budget, 14 * 86_400)
            .expect("按容量清理失败");

        assert_eq!(stats.removed_count, 1);
        assert!(store.get(&oldest_inactive.id).unwrap().is_none());
        assert!(!oldest_inactive.image_path.exists());
        assert!(store.get(&newer_inactive.id).unwrap().is_some());
        assert!(store.get(&active_large.id).unwrap().is_some());
    }

    #[test]
    fn privacy_delete_and_clear_history_remove_files_from_disk() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let store = PinStore::open_at(dir.path().join("pinshot")).expect("打开缓存失败");
        let cfg = CacheConfig::default();
        let png = make_test_png(80, 80, 3);

        let a = store
            .add_png_entry(
                NewEntryRequest {
                    kind: PinKind::ClipboardText,
                    png_bytes: &png,
                    text_content: Some("secret token"),
                    title_preview: "secret".to_string(),
                    source_scale: 1.0,
                    active: false,
                },
                &cfg,
            )
            .unwrap();

        let b = store
            .add_png_entry(
                NewEntryRequest {
                    kind: PinKind::Screenshot,
                    png_bytes: &png,
                    text_content: None,
                    title_preview: "active".to_string(),
                    source_scale: 1.0,
                    active: true,
                },
                &cfg,
            )
            .unwrap();

        // 单条物理删除
        assert!(store.delete_entry(&a.id).unwrap());
        assert!(!a.image_path.exists());
        assert!(!a.thumb_path.exists());
        assert!(!a.text_path.as_ref().unwrap().exists());

        // 清空非活跃历史不删活跃项
        let (removed, active_ids) = store.clear_history(false).unwrap();
        assert_eq!(removed, 0);
        assert!(active_ids.is_empty());
        assert!(store.get(&b.id).unwrap().is_some());

        // clear_history(true) 连同活跃项一并物理删除并返回活跃 ID
        let (removed_all, removed_active) = store.clear_history(true).unwrap();
        assert_eq!(removed_all, 1);
        assert_eq!(removed_active, vec![b.id.clone()]);
        assert!(!b.image_path.exists());
    }

    #[test]
    fn replace_entry_image_keeps_backup_history_and_updates_active_pin() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let store = PinStore::open_at(dir.path().join("pinshot")).expect("打开缓存失败");
        let cfg = CacheConfig::default();

        let orig_png = make_test_png(100, 80, 1);
        let edited_png = make_test_png(120, 90, 99);

        let orig = store
            .add_png_entry(
                NewEntryRequest {
                    kind: PinKind::Screenshot,
                    png_bytes: &orig_png,
                    text_content: None,
                    title_preview: "原截图".to_string(),
                    source_scale: 1.25,
                    active: true,
                },
                &cfg,
            )
            .unwrap();

        let updated = store
            .replace_entry_image(&orig.id, &edited_png, &cfg)
            .expect("标注回流替换失败");

        assert_eq!(updated.id, orig.id);
        assert_eq!((updated.width, updated.height), (120, 90));
        assert!(updated.active);

        let history = store.list_history(10).unwrap();
        assert_eq!(history.len(), 2, "应保留标注前版本作为非活跃历史");
        let backup = history.iter().find(|e| e.id != orig.id).unwrap();
        assert!(!backup.active);
        assert_eq!((backup.width, backup.height), (100, 80));
        assert!(backup.image_path.exists());
    }

    #[test]
    fn screencast_token_persists_with_0600_mode() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let store = PinStore::open_at(dir.path().join("pinshot")).expect("打开缓存失败");

        assert_eq!(store.load_screencast_token().unwrap(), None);
        store.save_screencast_token("tok-abc-123").unwrap();
        assert_eq!(
            store.load_screencast_token().unwrap().as_deref(),
            Some("tok-abc-123")
        );

        let mode = fs::metadata(store.screencast_token_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        store.clear_screencast_token().unwrap();
        assert_eq!(store.load_screencast_token().unwrap(), None);
    }
}
