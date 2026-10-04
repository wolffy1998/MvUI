//! Diagnostic: show exactly what `mame -help` writes, byte for byte.
//!
//! Usage: cargo run --release --example mame_help_probe -- <mame.exe>
//!
//! `MameBinary::detect` takes the *first non-empty line* of stdout as the
//! version string. On this machine the first line of `mame -help` is not the
//! version banner at all — it is a garbled non-ASCII line, and the real
//! `MAME v0.285 (unknown)` banner sits on line 1. This example exists to show
//! that without any shell/locale layer in between: it reads the raw bytes the
//! child produced and prints them as hex, as lossy UTF-8, and as the lines the
//! Rust code actually sees.

use std::process::{Command, Stdio};

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| r"D:\Game\MAME\MAME-0.284\mame.exe".to_string());

    let out = Command::new(&path)
        .arg("-help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    let out = match out {
        Ok(o) => o,
        Err(e) => {
            println!("spawn FAILED: {e}");
            return;
        }
    };

    println!("path:        {path}");
    println!("exit status: {:?}", out.status);
    println!("stdout:      {} bytes", out.stdout.len());
    println!("stderr:      {} bytes", out.stderr.len());

    let head = &out.stdout[..out.stdout.len().min(64)];
    let hex: Vec<String> = head.iter().map(|b| format!("{b:02x}")).collect();
    println!("\nfirst {} stdout bytes, hex:\n{}", head.len(), hex.join(" "));

    // Is the very first thing a UTF-8 BOM?
    println!(
        "starts with UTF-8 BOM: {}",
        out.stdout.starts_with(&[0xEF, 0xBB, 0xBF])
    );

    // What `detect()` sees: `String::from_utf8_lossy(stdout).lines()`,
    // trimmed, first non-empty.
    let lossy = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = lossy.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    println!("\n--- lines as `detect()` sees them (utf8_lossy + trim) ---");
    for (i, l) in lines.iter().take(6).enumerate() {
        println!("[{i}] {l:?}");
    }

    let detected = lines.first().copied().unwrap_or("");
    println!("\ndetect() would return: {detected:?}");
    let banner = lines.iter().find(|l| l.starts_with("MAME v"));
    println!("first line starting with \"MAME v\": {banner:?}");

    // The same bytes decoded as the system code page — GBK/936 on this box.
    #[cfg(target_os = "windows")]
    {
        let ansi = decode_acp(&out.stdout);
        println!("\n--- first 3 lines decoded as system code page (ACP) ---");
        for (i, l) in ansi.lines().take(3).enumerate() {
            println!("[{i}] {l:?}");
        }
    }
}

/// Decode bytes using the active Windows code page (what a console would show).
#[cfg(target_os = "windows")]
fn decode_acp(bytes: &[u8]) -> String {
    use std::ffi::c_int;
    extern "system" {
        fn MultiByteToWideChar(
            code_page: u32,
            dw_flags: u32,
            lp_multibyte_str: *const u8,
            cb_multibyte: c_int,
            lp_wide_char_str: *mut u16,
            cch_wide_char: c_int,
        ) -> c_int;
    }
    const CP_ACP: u32 = 0;
    unsafe {
        let need = MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), bytes.len() as c_int, std::ptr::null_mut(), 0);
        if need <= 0 {
            return String::new();
        }
        let mut wide = vec![0u16; need as usize];
        let got = MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), bytes.len() as c_int, wide.as_mut_ptr(), need);
        String::from_utf16_lossy(&wide[..got.max(0) as usize])
    }
}
