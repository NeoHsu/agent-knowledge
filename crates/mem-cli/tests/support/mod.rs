#![allow(dead_code)]

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};

pub struct TestRepo {
    path: PathBuf,
}

pub struct TestRuntimeStore {
    install_dir: PathBuf,
    run_dir: PathBuf,
    home: PathBuf,
    mem: PathBuf,
}

impl TestRepo {
    pub fn new(name: &str) -> Self {
        Self {
            path: temp_repo(name),
        }
    }

    #[allow(dead_code)]
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        self.path.join(path)
    }

    pub fn run(&self, args: &[&str]) -> String {
        run(&self.path, args)
    }

    #[allow(dead_code)]
    pub fn run_fail(&self, args: &[&str]) -> String {
        run_fail(&self.path, args)
    }

    pub fn run_fail_with_env(&self, args: &[&str], key: &str, value: &str) -> String {
        let output = repo_command(&self.path, args)
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
        insert_raw_memory(&self.path, id, name, content);
    }
}

impl TestRuntimeStore {
    pub fn new(name: &str) -> Self {
        let root = temp_path(name);
        let install_dir = root.join("install");
        let run_dir = root.join("run");
        let home = root.join("home");
        fs::create_dir_all(&install_dir).expect("install dir");
        fs::create_dir_all(&run_dir).expect("run dir");
        fs::create_dir_all(&home).expect("home dir");
        let mem = install_dir.join("mem");
        fs::copy(mem_bin(), &mem).expect("copy mem binary");
        Self {
            install_dir,
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

impl Drop for TestRepo {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).ok();
    }
}

impl Drop for TestRuntimeStore {
    fn drop(&mut self) {
        if let Some(root) = self.install_dir.parent() {
            fs::remove_dir_all(root).ok();
        }
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

pub fn temp_path(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    std::env::temp_dir().join(format!("mnemark-{name}-{stamp}"))
}

pub fn temp_repo(name: &str) -> PathBuf {
    let dir = temp_path(name);
    fs::create_dir_all(dir.join("schema")).expect("schema dir");
    fs::write(
        dir.join("schema/memory-schema.sql"),
        include_str!("../../../../schema/memory-schema.sql"),
    )
    .expect("schema");
    dir
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
