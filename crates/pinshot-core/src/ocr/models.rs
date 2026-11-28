use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{OcrConfig, expand_tilde};
use crate::error::{PinshotError, Result};

/// 内置默认离线 OCR 模型 ID
pub const DEFAULT_MODEL_ID: &str = "ppocrv5-mobile";

const DEFAULT_HF_ENDPOINT: &str = "https://huggingface.co";

/// 单个模型文件描述（支持 SHA-256 强校验）
#[derive(Debug, Clone, Copy)]
pub struct ModelFileSpec {
    /// 相对模型根目录路径，例如 `det/inference.onnx`
    pub rel_path: &'static str,
    /// Hugging Face 仓库及文件相对路径（拼接在 `HF_ENDPOINT` 后）
    pub hf_path: &'static str,
    /// 期望的 SHA-256（小写十六进制）
    pub sha256: &'static str,
}

/// 内置模型配置（预留扩展多个 model_id）
#[derive(Debug, Clone, Copy)]
pub struct BuiltinModelSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub files: &'static [ModelFileSpec],
}

const PPOCRV5_MOBILE_FILES: &[ModelFileSpec] = &[
    ModelFileSpec {
        rel_path: "det/inference.onnx",
        hf_path: "PaddlePaddle/PP-OCRv5_mobile_det_onnx/resolve/main/inference.onnx",
        sha256: "a431985659dc921974177a95adcfbb90fd9e51989a5e04d70d0b75f597b6e61d",
    },
    ModelFileSpec {
        rel_path: "det/inference.yml",
        hf_path: "PaddlePaddle/PP-OCRv5_mobile_det_onnx/resolve/main/inference.yml",
        sha256: "98069072e1b6b37d727fd9d9f11725faa46d6ea0de012f2ed26caea011c37699",
    },
    ModelFileSpec {
        rel_path: "rec/inference.onnx",
        hf_path: "PaddlePaddle/PP-OCRv5_mobile_rec_onnx/resolve/main/inference.onnx",
        sha256: "da72dc72ca4dc220df0dfde68c1dedc31c58d3e76a25871122e5056227d50092",
    },
    ModelFileSpec {
        rel_path: "rec/inference.yml",
        hf_path: "PaddlePaddle/PP-OCRv5_mobile_rec_onnx/resolve/main/inference.yml",
        sha256: "5dfeb2777f6d0db8177d8128a8acfcf6e6276dc4ac73ea3bf0dc06d6a5e85d8e",
    },
];

pub const BUILTIN_MODELS: &[BuiltinModelSpec] = &[BuiltinModelSpec {
    id: DEFAULT_MODEL_ID,
    display_name: "PP-OCRv5 Mobile (中英日/代码)",
    files: PPOCRV5_MOBILE_FILES,
}];

/// 已定位到的完整模型目录
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModelDir {
    pub model_id: String,
    pub root: PathBuf,
    pub det_dir: PathBuf,
    pub rec_dir: PathBuf,
    pub source: ModelLocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelLocation {
    Custom,
    User,
    System,
}

pub fn find_builtin_spec(model_id: &str) -> Option<&'static BuiltinModelSpec> {
    BUILTIN_MODELS.iter().find(|spec| spec.id == model_id)
}

/// 返回用户级 OCR 模型根目录：`${XDG_DATA_HOME:-~/.local/share}/pinshot/ocr`
pub fn user_models_root() -> Result<PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| PinshotError::HomeDir("无法获取 XDG data 目录".to_string()))?;
    Ok(base.join("pinshot").join("ocr"))
}

/// 检查目录是否包含完整的 `det` 与 `rec` 模型及配置文件
pub fn is_valid_model_dir(root: &Path) -> bool {
    let det = root.join("det");
    let rec = root.join("rec");
    det.join("inference.onnx").is_file()
        && det.join("inference.yml").is_file()
        && rec.join("inference.onnx").is_file()
        && rec.join("inference.yml").is_file()
}

/// 按「自定义目录 → 用户目录 → 系统目录」顺序解析当前配置的 OCR 模型
pub fn resolve_model_dir(cfg: &OcrConfig) -> Result<Option<ResolvedModelDir>> {
    let model_id = if cfg.model.trim().is_empty() {
        DEFAULT_MODEL_ID.to_string()
    } else {
        cfg.model.trim().to_string()
    };

    let mut candidates: Vec<(PathBuf, ModelLocation)> = Vec::new();
    if !cfg.model_dir.trim().is_empty() {
        let custom = expand_tilde(cfg.model_dir.trim())?;
        candidates.push((custom.clone(), ModelLocation::Custom));
        candidates.push((custom.join(&model_id), ModelLocation::Custom));
    }
    if let Ok(user_root) = user_models_root() {
        candidates.push((user_root.join(&model_id), ModelLocation::User));
    }
    candidates.push((
        PathBuf::from("/usr/share/pinshot/ocr").join(&model_id),
        ModelLocation::System,
    ));
    candidates.push((
        PathBuf::from("/usr/local/share/pinshot/ocr").join(&model_id),
        ModelLocation::System,
    ));

    for (root, source) in candidates {
        if is_valid_model_dir(&root) {
            return Ok(Some(ResolvedModelDir {
                model_id,
                det_dir: root.join("det"),
                rec_dir: root.join("rec"),
                root,
                source,
            }));
        }
    }

    Ok(None)
}

/// 统计模型目录当前占用的总字节数
pub fn model_dir_size_bytes(root: &Path) -> u64 {
    [
        "det/inference.onnx",
        "det/inference.yml",
        "rec/inference.onnx",
        "rec/inference.yml",
    ]
    .iter()
    .filter_map(|rel| fs::metadata(root.join(rel)).ok())
    .map(|meta| meta.len())
    .sum()
}

/// 下载并校验内置模型到目标根目录（支持 `HF_ENDPOINT` 环境变量切换镜像）
pub fn download_builtin_model(model_id: &str, dest_root: &Path) -> Result<PathBuf> {
    let spec = find_builtin_spec(model_id)
        .ok_or_else(|| PinshotError::Ocr(format!("未知的内置 OCR 模型: {model_id}")))?;

    let endpoint = env::var("HF_ENDPOINT")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_HF_ENDPOINT.to_string());

    let model_root = dest_root.join(spec.id);
    for file in spec.files {
        let target = model_root.join(file.rel_path);
        if target.is_file() && verify_file_sha256(&target, file.sha256)? {
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| PinshotError::io(parent, e))?;
        }
        let part = target.with_extension("part");
        let url = format!("{endpoint}/{}", file.hf_path);

        let output = Command::new("curl")
            .args([
                "-fsSL",
                "-C",
                "-",
                "--retry",
                "5",
                "--retry-delay",
                "2",
                "--retry-all-errors",
                &url,
                "-o",
            ])
            .arg(&part)
            .output()
            .map_err(|e| PinshotError::ExternalCommand {
                cmd: "curl".to_string(),
                detail: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(PinshotError::Ocr(format!(
                "下载模型文件 {} 失败（可设置 HF_ENDPOINT 镜像重试）: {}",
                file.rel_path,
                if stderr.is_empty() {
                    output.status.to_string()
                } else {
                    stderr
                }
            )));
        }

        if !verify_file_sha256(&part, file.sha256)? {
            let _ = fs::remove_file(&part);
            return Err(PinshotError::Ocr(format!(
                "模型文件 {} 的 SHA-256 校验失败，已丢弃损坏文件",
                file.rel_path
            )));
        }

        fs::rename(&part, &target).map_err(|e| PinshotError::io(&target, e))?;
    }

    Ok(model_root)
}

/// 纯 Rust SHA-256 文件校验（零额外依赖）
pub fn verify_file_sha256(path: &Path, expected_hex: &str) -> Result<bool> {
    let actual = sha256_file_hex(path)?;
    Ok(actual.eq_ignore_ascii_case(expected_hex))
}

pub fn sha256_file_hex(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|e| PinshotError::io(path, e))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| PinshotError::io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    Ok(hex)
}

/// 精简纯 Rust FIPS 180-4 SHA-256 实现
struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_len: usize,
    total_bits: u64,
}

impl Sha256 {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: [0u8; 64],
            buffer_len: 0,
            total_bits: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.total_bits = self.total_bits.wrapping_add((data.len() as u64) * 8);
        if self.buffer_len > 0 {
            let take = (64 - self.buffer_len).min(data.len());
            self.buffer[self.buffer_len..self.buffer_len + take].copy_from_slice(&data[..take]);
            self.buffer_len += take;
            data = &data[take..];
            if self.buffer_len == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffer_len = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.compress(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffer_len = data.len();
        }
    }

    fn finalize(mut self) -> [u8; 32] {
        let total_bits = self.total_bits;
        self.buffer[self.buffer_len] = 0x80;
        self.buffer_len += 1;
        if self.buffer_len > 56 {
            self.buffer[self.buffer_len..64].fill(0);
            let block = self.buffer;
            self.compress(&block);
            self.buffer_len = 0;
        }
        self.buffer[self.buffer_len..56].fill(0);
        self.buffer[56..64].copy_from_slice(&total_bits.to_be_bytes());
        let block = self.buffer;
        self.compress(&block);

        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for (&k, &word) in Self::K.iter().zip(w.iter()) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(k)
                .wrapping_add(word);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn sha256_matches_standard_test_vectors() {
        let dir = TempDir::new().unwrap();
        let empty = dir.path().join("empty");
        fs::write(&empty, b"").unwrap();
        assert_eq!(
            sha256_file_hex(&empty).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        let abc = dir.path().join("abc");
        fs::write(&abc, b"abc").unwrap();
        assert_eq!(
            sha256_file_hex(&abc).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn resolve_model_dir_prefers_custom_directory() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("custom-model");
        fs::create_dir_all(root.join("det")).unwrap();
        fs::create_dir_all(root.join("rec")).unwrap();
        for rel in [
            "det/inference.onnx",
            "det/inference.yml",
            "rec/inference.onnx",
            "rec/inference.yml",
        ] {
            fs::write(root.join(rel), b"stub").unwrap();
        }

        let cfg = OcrConfig {
            model: "custom".to_string(),
            model_dir: root.to_string_lossy().into_owned(),
            ..OcrConfig::default()
        };
        let resolved = resolve_model_dir(&cfg)
            .unwrap()
            .expect("应定位到自定义模型目录");
        assert_eq!(resolved.source, ModelLocation::Custom);
        assert_eq!(resolved.root, root);
        assert_eq!(model_dir_size_bytes(&resolved.root), 16);
    }
}
