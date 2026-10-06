//! 封装 mame.exe 子进程（origin: processmanager.cpp +
//! utils.cpp 的 getMameVersion）。

use crate::dlog;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, thiserror::Error)]
pub enum MameError {
    #[error("mame binary not found: {0}")]
    NotFound(String),
    #[error("failed to spawn mame: {0}")]
    Spawn(String),
    #[error("mame invocation failed: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct MameBinary {
    pub path: PathBuf,
    pub version: String,
}

impl MameBinary {
    /// origin: utils->getMameVersion —— 跑一次 `mame -help` 并留下那行
    /// 版本横幅。
    pub fn detect(path: &Path) -> Result<Self, MameError> {
        if !path.exists() {
            return Err(MameError::NotFound(path.display().to_string()));
        }
        let mut cmd = quiet_command(path);
        cmd.arg("-help");
        let out = cmd.output();
        let version = match out {
            Ok(o) => version_from_help(&String::from_utf8_lossy(&o.stdout)),
            Err(e) => return Err(e.into()),
        };
        dlog!("mame: 探测 {} → 版本 {:?}", path.display(), version);
        Ok(Self {
            path: path.to_path_buf(),
            version,
        })
    }

    pub fn spawn_listxml(&self) -> Result<Child, MameError> {
        self.run(&["-listxml"])
    }

    /// origin: 第二个子进程 —— 选项模板的来源
    pub fn spawn_showconfig(&self) -> Result<Child, MameError> {
        self.run(&["-showconfig", "-noreadconfig"])
    }

    pub fn spawn_verifyroms(&self, game: Option<&str>) -> Result<Child, MameError> {
        match game {
            Some(g) => self.run(&["-verifyroms", g]),
            None => self.run(&["-verifyroms"]),
        }
    }

    pub fn spawn_verifysamples(&self, game: Option<&str>) -> Result<Child, MameError> {
        match game {
            Some(g) => self.run(&["-verifysamples", g]),
            None => self.run(&["-verifysamples"]),
        }
    }

    pub fn spawn_run(&self, args: &[String]) -> Result<Child, MameError> {
        self.run(args.iter().map(|s| s.as_str()).collect::<Vec<_>>().as_slice())
    }

    fn run(&self, args: &[&str]) -> Result<Child, MameError> {
        let mut cmd = quiet_command(&self.path);
        cmd.args(args);
        // 记下实际启动的进程与参数：校验/解析出问题的时候，boot.log
        // 里这一行能直接说明 MvUI 到底让 mame 干了什么。
        dlog!("mame: 启动 {} {:?}", self.path.display(), args);
        cmd.spawn()
            .map_err(|e| MameError::Spawn(format!("{} {:?}: {e}", self.path.display(), args)))
    }
}

/// 构造一个绝不会闪出控制台窗口的 `Command`。
///
/// `detect()` 曾经就地拼命令，因此每次启动都会弹出一个控制台窗口
/// （origin: processmanager.cpp 是以分离方式跑这些检查的）。
fn quiet_command(path: &Path) -> Command {
    let mut cmd = Command::new(path);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// 从 `mame -help` 的输出里取出版本横幅那一行。
///
/// 以前的做法是"第一个非空行"，1.8.2 原版也是这么做的——但对于一个
/// 会在 MAME 自己的横幅之前先打出自家横幅的构建，两者都会错。参考机
/// 上就遇到了（一个 0.285 的 "ARCADE" 改包），它的 `mame -help`
/// 是这样开头的：
///
/// ```text
/// B绔?绾㈢櫧鏈烘椂绌鸿埍 鏁村悎      <- 第 0 行
/// MAME v0.285 (unknown)            <- 第 1 行
/// Copyright MAMEdev and contributors
/// ```
///
/// 第 0 行是改包加的本地化标题。它的字节是**合法 UTF-8**，只是解出来
/// 是乱码，所以这不是我们这边的解码 bug——是生产者把 GBK 字节写进了
/// 一个它自己声明为 UTF-8 的输出流。取第 0 行于是会得到
/// `B绔?绾㈢櫧鏈烘椂绌鸿埍 鏁村悎` 这样的版本串，它随后被存进
/// `gamelist.cache` 的版本戳，并原样显示在"关于"框里。1.8.2 自己的
/// 注释列了四种可接受的横幅形态，其中就包括 `ARCADE v0.289.0`——它
/// 只是没有防备这种横幅**排在第一个**的情况。
///
/// 所以：优先挑真正像版本横幅的那一行，只有在没有任何一行像的时候
/// 才退回"第一个非空行"（一个故意改过名的构建仍然该产出*某个*版本
/// 串，而不是空串——空串会被 `try_accept_mame` 当成"无效的二进制"）。
fn version_from_help(help: &str) -> String {
    let lines = || help.lines().map(|l| l.trim()).filter(|l| !l.is_empty());

    // 1) 标准横幅：`MAME v0.285 (unknown)`、`M.A.M.E. v0.168 (...)`，
    //    或者 nightly 的 `nightly build: MAME v0.172 (699-g5d1ce79)`。
    if let Some(l) = lines().find(|l| looks_like_version_banner(l)) {
        return l.to_string();
    }
    // 2) 没有可识别的横幅 —— 保留历史行为，好让一个不寻常的构建也能
    //    拿到*某个*版本，而不是被直接拒掉。
    lines().next().unwrap_or("").to_string()
}

/// 这一行读起来像 MAME 的版本横幅吗？
///
/// 判据是词边界上的 `v<数字>`——这正是把横幅与帮助正文（"This
/// software reproduces, more or less faithfully…"）以及改包自己的标题
/// 行区分开的东西。如果还要求出现 "MAME" 这个词，就会把
/// `ARCADE v0.289.0` 拒掉，而那是 1.8.2 明确列出的可接受的横幅形态，
/// 所以名字是故意**不**要求的。
fn looks_like_version_banner(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.char_indices().any(|(i, c)| {
        c == 'v'
            && i + 1 < lower.len()
            && lower.as_bytes()[i + 1].is_ascii_digit()
            // word boundary before the `v`, so "via"/"video0" don't match
            && (i == 0 || !lower.as_bytes()[i - 1].is_ascii_alphanumeric())
    })
}

/// 边到边读子进程 stdout 的每一行，喂给回调。
///
/// 会阻塞到 EOF —— 请从后台线程调用。
pub fn pump_lines(child: &mut Child, mut on_line: impl FnMut(String)) -> Option<i32> {
    let stdout = child.stdout.take()?;
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        match line {
            Ok(l) => on_line(l),
            Err(_) => break,
        }
    }
    let code = match child.wait() {
        Ok(status) => status.code(),
        Err(_) => None,
    };
    dlog!("mame: 子进程结束，退出码 {:?}", code);
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 参考机上真实的 `mame -help` 开头，逐字照抄——包括第 0 行那条
    /// 乱码的改包标题。之所以照抄而不是改写：重点就在于第 0 行是
    /// **合法 UTF-8**，用一个手写的占位符造出来的测试抓不到"退回
    /// 第一个非空行"这种回归。
    const REPACK_HELP: &str = "B绔?绾㈢櫧鏈烘椂绌鸿埍 鏁村悎\r\n\
                               MAME v0.285 (unknown)\r\n\
                               Copyright MAMEdev and contributors\r\n\
                               \r\n\
                               This software reproduces, more or less faithfully, the behaviour of a wide range";

    #[test]
    fn repack_banner_is_skipped_for_the_real_version_line() {
        assert_eq!(version_from_help(REPACK_HELP), "MAME v0.285 (unknown)");
    }

    /// 1.8.2 记录的四种形态，每一种都必须仍然被认出来。
    #[test]
    fn all_documented_banner_shapes_are_recognised() {
        for (line, want) in [
            ("M.A.M.E. v0.168 (Mar 15 2016)", "M.A.M.E. v0.168 (Mar 15 2016)"),
            ("nightly build: MAME v0.172 (699-g5d1ce79)", "nightly build: MAME v0.172 (699-g5d1ce79)"),
            ("MAME v0.173", "MAME v0.173"),
            ("ARCADE v0.289.0 (2026-07-31)", "ARCADE v0.289.0 (2026-07-31)"),
        ] {
            let help = format!("Some repack title\r\n{line}\r\nrest of the help");
            assert_eq!(version_from_help(&help), want, "banner {line:?} was not picked");
        }
    }

    /// `ARCADE v0.289.0` 里面没有 "mame" 这个词，但照样必须胜出——
    /// 它正是记载中的第 4 种形态，而且参考机上那个改包打印的就是
    /// 这种行。
    #[test]
    fn version_banner_without_the_word_mame_is_accepted() {
        assert!(looks_like_version_banner("ARCADE v0.289.0 (2026-07-31)"));
    }

    /// 标题行不能被误当成横幅。`v` 这个启发式要求紧跟一个数字，所以
    /// "ACME Video" 会被排除在外。
    #[test]
    fn ordinary_help_text_is_not_a_banner() {
        assert!(!looks_like_version_banner("This software reproduces, more or less faithfully"));
        assert!(!looks_like_version_banner("MAME is distributed via video channels"));
        assert!(!looks_like_version_banner("B绔?绾㈢櫧鏈烘椂绌鸿埍 鏁村悎"));
    }

    /// 完全没有横幅时：退回第一个非空行，而不是返回空串——调用方会
    /// 把空串读成"这不是个能用的二进制"。
    #[test]
    fn unknown_output_falls_back_to_the_first_line() {
        let help = "\r\n   \r\nweird build, no banner\r\nmore";
        assert_eq!(version_from_help(help), "weird build, no banner");
        assert_eq!(version_from_help(""), "");
    }
}
