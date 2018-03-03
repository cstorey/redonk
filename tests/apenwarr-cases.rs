#[macro_use]
extern crate error_chain;
extern crate tempdir;
extern crate walkdir;
use tempdir::TempDir;
use walkdir::WalkDir;
use std::fs;
use std::io::{self, BufRead};
use std::path::Path;
use std::process::Command;
use std::path;
use std::env;

const EXE_DIR: &'static str = "target/debug";

error_chain! {
    foreign_links {
        Io(::std::io::Error);
        JoinPaths(env::JoinPathsError);
    }
}

fn copy_dir<P0: AsRef<Path>, P1: AsRef<Path>>(src: P0, dst: P1) -> Result<()> {
    for e in WalkDir::new(&src) {
        let e = e.chain_err(|| "walkdir")?;

        let path = e.path()
            .strip_prefix(&src)
            .chain_err(|| "path from test case")?;
        let dest = dst.as_ref().join(path);
        // println!("{:?} → {:?}", e.path(), dest);

        if e.file_type().is_dir() {
            fs::create_dir(&dest).chain_err(|| "mkdir")?;
        } else if e.file_type().is_file() {
            fs::copy(&e.path(), &dest).chain_err(|| "copy file")?;
        } else {
            panic!("Unrecognised fs entity: {:?}: {:?}", e.path(), e.metadata())
        }
    }

    Ok(())
}

#[derive(Debug)]
struct TestCase {
    tmpdir: path::PathBuf,
}

impl TestCase {
    fn new(example: &str) -> Result<TestCase> {
        let tmpdir = TempDir::new(example).chain_err(|| "TempDir::new")?;
        let basedir = "t";
        fs::remove_dir_all(&tmpdir.path()).chain_err(|| "cleanup")?;
        copy_dir(&basedir, &tmpdir.path()).chain_err(|| "copy_dir")?;

        Ok(TestCase {
            tmpdir: tmpdir.into_path().join(example),
        })
    }

    fn run(&self) -> Result<()> {
        let cwd = env::current_dir()?;
        let exec_dir = cwd.join(EXE_DIR);
        let curr_path = env::var_os("PATH").chain_err(|| "lookup current $PATH")?;
        let mut paths = env::split_paths(&curr_path).collect::<Vec<_>>();
        paths.insert(0, exec_dir.clone());

        let mut cmd = Command::new(exec_dir.join("redonk"));
        cmd.arg("redo");
        cmd.arg("all");
        cmd.current_dir(&self.tmpdir);
        cmd.env("PATH", env::join_paths(paths)?);

        let child = cmd.spawn()
            .chain_err(|| format!("Command::spawn: {:?}", cmd))?
            .wait()
            .chain_err(|| format!("Child::wait: {:?}", cmd))?;

        if child.success() {
            Ok(())
        } else {
            Err(format!("Child command: {:?} exited: {:?}", cmd, child).into())
        }
    }
}

#[test]
fn t_000_set_minus_e() {
    let tc = TestCase::new("000-set-minus-e").expect("setup");
    tc.run().expect("000-set-minus-e");

    println!("Test case dir: {:?}", tc);
    let log = io::BufReader::new(fs::File::open(tc.tmpdir.join("log")).expect("log file"));

    let log_content = log.lines()
        .map(|r| r.map_err(|e| e.into()))
        .collect::<Result<Vec<_>>>()
        .expect("log lines");
    assert_eq!(log_content, vec!["ok"]);
}

#[test]
#[should_panic(expected = "Child command:")]
fn t_100_args() {
    let tc = TestCase::new("100-args").expect("setup");
    tc.run().expect("100-args");
}

#[test]
fn t_110_compile() {
    let tc = TestCase::new("110-compile").expect("setup");
    tc.run().expect("110-compile");

    let hello = tc.tmpdir.join("hello");

    println!("Test case dir: {:?}", tc);

    let _ = fs::metadata(&hello)
        .chain_err(|| format!("Built hello at {:?}", hello))
        .expect("hello");
    let res = Command::new(&hello)
        .spawn()
        .expect("spawn hello")
        .wait()
        .expect("wait hello");
    assert!(res.success(), "Compiled hello ({:?}) ran okay", hello);
}
