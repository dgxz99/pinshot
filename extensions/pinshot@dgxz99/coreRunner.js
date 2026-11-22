import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

Gio._promisify(
    Gio.Subprocess.prototype,
    'communicate_utf8_async',
    'communicate_utf8_finish'
);

/**
 * 解析 pinshot CLI 可执行文件路径（优先 ~/.local/bin/pinshot，其次依赖 PATH）
 */
function resolvePinshotBinary() {
    const localBin = GLib.build_filenamev([GLib.get_home_dir(), '.local', 'bin', 'pinshot']);
    if (GLib.file_test(localBin, GLib.FileTest.IS_EXECUTABLE)) {
        return localBin;
    }
    const inPath = GLib.find_program_in_path('pinshot');
    if (inPath) {
        return inPath;
    }
    return localBin;
}

/**
 * 异步执行 pinshot 子命令并返回标准输出，绝不阻塞 GNOME Shell 主循环
 */
export async function runPinshotAsync(args) {
    const bin = resolvePinshotBinary();
    const argv = [bin, ...args];

    try {
        const proc = Gio.Subprocess.new(
            argv,
            Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE
        );
        const [stdout, stderr] = await proc.communicate_utf8_async(null, null);
        const ok = proc.get_successful();
        return {
            ok,
            stdout: stdout ?? '',
            stderr: stderr ?? '',
        };
    } catch (err) {
        console.error(`[pinshot] 启动子进程失败 (${argv.join(' ')}): ${err}`);
        return {
            ok: false,
            stdout: '',
            stderr: String(err),
        };
    }
}

/**
 * 触发即走的异步内部回调命令，失败时输出日志上下文
 */
export function firePinshot(args) {
    runPinshotAsync(args)
        .then(result => {
            if (!result.ok) {
                const detail = result.stderr.trim() || '未知错误';
                console.warn(`[pinshot] 子命令失败 (${args.join(' ')}): ${detail}`);
            }
        })
        .catch(err => {
            console.error(`[pinshot] 子命令异常 (${args.join(' ')}): ${err}`);
        });
}
