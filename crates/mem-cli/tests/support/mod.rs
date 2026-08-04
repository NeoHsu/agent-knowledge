#![allow(dead_code)]

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::Duration;

use rusqlite::{Connection, params};
use tempfile::TempDir;

pub struct TestRepo {
    directory: TempDir,
}

pub struct TestRuntimeStore {
    _root: TempDir,
    run_dir: PathBuf,
    home: PathBuf,
    mem: PathBuf,
}

impl TestRepo {
    pub fn new(name: &str) -> Self {
        let directory = tempfile::Builder::new()
            .prefix(&format!("mnemark-{name}-"))
            .tempdir()
            .expect("test repository");
        seed_repo(directory.path());
        Self { directory }
    }

    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        self.directory.path()
    }

    pub fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        self.path().join(path)
    }

    pub fn run(&self, args: &[&str]) -> String {
        run(self.path(), args)
    }

    #[allow(dead_code)]
    pub fn run_fail(&self, args: &[&str]) -> String {
        run_fail(self.path(), args)
    }

    pub fn run_fail_with_env(&self, args: &[&str], key: &str, value: &str) -> String {
        let output = repo_command(self.path(), args)
            .env(key, value)
            .output()
            .expect("run mem with environment");
        assert!(
            !output.status.success(),
            "command unexpectedly succeeded: {:?}\nstdout={}\nstderr={}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }

    pub fn insert_raw_memory(&self, id: &str, name: &str, content: &str) {
        insert_raw_memory(self.path(), id, name, content);
    }
}

impl TestRuntimeStore {
    pub fn new(name: &str) -> Self {
        let root = tempfile::Builder::new()
            .prefix(&format!("mnemark-{name}-"))
            .tempdir()
            .expect("runtime test root");
        let install_dir = root.path().join("install");
        let run_dir = root.path().join("run");
        let home = root.path().join("home");
        fs::create_dir_all(&install_dir).expect("install dir");
        fs::create_dir_all(&run_dir).expect("run dir");
        fs::create_dir_all(&home).expect("home dir");
        let mem = install_dir.join("mem");
        fs::copy(mem_bin(), &mem).expect("copy mem binary");
        Self {
            _root: root,
            run_dir,
            home,
            mem,
        }
    }

    pub fn home(&self) -> &PathBuf {
        &self.home
    }

    pub fn run_dir(&self) -> &PathBuf {
        &self.run_dir
    }

    pub fn run(&self, args: &[&str]) -> String {
        let output = run_installed_mem(&self.mem, &self.run_dir, &self.home, args);
        assert!(
            output.status.success(),
            "command failed: {:?}\nstdout={}\nstderr={}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf8 stdout")
    }

    pub fn run_fail(&self, args: &[&str]) -> String {
        let output = run_installed_mem(&self.mem, &self.run_dir, &self.home, args);
        assert!(
            !output.status.success(),
            "command unexpectedly succeeded: {:?}\nstdout={}\nstderr={}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }
}

fn run_installed_mem(mem: &Path, run_dir: &Path, home: &Path, args: &[&str]) -> Output {
    for attempt in 0..5 {
        match Command::new(mem)
            .current_dir(run_dir)
            .env("MNEMARK_HOME", home)
            .args(args)
            .output()
        {
            Ok(output) => return output,
            Err(error) if error.kind() == ErrorKind::ExecutableFileBusy && attempt < 4 => {
                thread::sleep(Duration::from_millis(25 * (attempt + 1)))
            }
            Err(error) => panic!("run installed mem: {error:?}"),
        }
    }
    unreachable!("retry loop always returns or panics")
}

pub fn mem_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mem"))
}

/// Build credential-shaped test data at runtime so source-tree secret scanners
/// do not mistake synthetic fixtures for committed credentials.
pub fn synthetic_github_token() -> String {
    ["ghp", "_", "abcdefghijklmnop", "1234567890"].concat()
}

pub fn synthetic_generic_secret() -> String {
    ["token", ": ", "abcdefgh", "12345678", "\n"].concat()
}

pub fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("mnemark-{name}-{}", uuid::Uuid::new_v4().simple()))
}

fn seed_repo(dir: &Path) {
    fs::create_dir_all(dir.join("schema")).expect("schema dir");
    fs::write(
        dir.join("schema/memory-schema.sql"),
        include_str!("../../../../schema/memory-schema.sql"),
    )
    .expect("schema");
}

fn repo_command(repo: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(mem_bin());
    command.current_dir(repo);
    if !args.contains(&"--home") {
        command.arg("--home").arg(repo);
    }
    command.args(args);
    command
}

pub fn run(repo: &Path, args: &[&str]) -> String {
    let output = repo_command(repo, args).output().expect("run mem");
    assert!(
        output.status.success(),
        "command failed: {:?}\nstdout={}\nstderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

#[allow(dead_code)]
pub fn run_fail(repo: &Path, args: &[&str]) -> String {
    let output = repo_command(repo, args).output().expect("run mem");
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: {:?}\nstdout={}\nstderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

pub fn insert_raw_memory(repo: &Path, id: &str, name: &str, content: &str) {
    let conn = Connection::open(repo.join("memory.db")).expect("open memory db");
    conn.execute(
        "INSERT INTO memories
        (id, type, name, content, tags, scope, source, confidence, protected, created_at, updated_at)
        VALUES (?1, 'reference', ?2, ?3, '[]', 'global', 'manual', 'high', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        params![id, name, content],
    )
    .expect("insert raw memory");
}
