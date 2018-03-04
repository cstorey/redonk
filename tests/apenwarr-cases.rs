#[macro_use]
extern crate error_chain;
extern crate tempdir;
extern crate walkdir;
use tempdir::TempDir;
use walkdir::WalkDir;
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::env;

const EXE_DIR: &'static str = "target/debug";

error_chain! {
    foreign_links {
        Io(::std::io::Error);
        JoinPaths(env::JoinPathsError);
    }
}

macro_rules! example {
    ($func: ident, $name: expr) => {
        example!($func, $name, );
    };
    ($func: ident, $name: expr, $( #[$meta:meta] )*) => {
        #[test]
        $(#[$meta])*
        fn $func() {
            let tc = TestCase::new($name).expect("setup");
            tc.run().expect($name);
        }
    };
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
    tmpdir: PathBuf,
    example: String,
}

impl TestCase {
    fn new(example: &str) -> Result<TestCase> {
        let tmpdir = TempDir::new(example).chain_err(|| "TempDir::new")?;
        let basedir = "t";
        fs::remove_dir_all(&tmpdir.path()).chain_err(|| "cleanup")?;
        copy_dir(&basedir, &tmpdir.path()).chain_err(|| "copy_dir")?;

        Ok(TestCase {
            tmpdir: tmpdir.into_path(),
            example: example.to_owned(),
        })
    }

    fn run(&self) -> Result<()> {
        let cwd = env::current_dir()?;
        let exec_dir = cwd.join(EXE_DIR);
        let curr_path = env::var_os("PATH").chain_err(|| "lookup current $PATH")?;
        let mut paths = env::split_paths(&curr_path).collect::<Vec<_>>();
        paths.insert(0, exec_dir.clone());

        let stdout_name = PathBuf::from(format!("target/{}.out.txt", self.example));
        let stderr_name = PathBuf::from(format!("target/{}.err.txt", self.example));

        let mut cmd = Command::new(exec_dir.join("redonk"));
        cmd.arg("redo");
        cmd.arg(PathBuf::from(&self.example).join("all"));
        cmd.current_dir(&self.tmpdir);
        cmd.env("PATH", env::join_paths(paths)?);
        cmd.stdout(fs::File::create(&stdout_name)
            .chain_err(|| stdout_name.to_string_lossy().into_owned())?);
        cmd.stderr(fs::File::create(&stderr_name)
            .chain_err(|| stderr_name.to_string_lossy().into_owned())?);
        println!("Child stdout: {:?}; stderr: {:?}", stdout_name, stderr_name);

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

    fn example_dir(&self) -> PathBuf {
        self.tmpdir.join(&self.example)
    }
}

#[test]
fn t_000_set_minus_e() {
    let tc = TestCase::new("000-set-minus-e").expect("setup");
    tc.run().expect("000-set-minus-e");

    println!("Test case dir: {:?}", tc);
    let log = io::BufReader::new(fs::File::open(tc.example_dir().join("log")).expect("log file"));

    let log_content = log.lines()
        .map(|r| r.map_err(|e| e.into()))
        .collect::<Result<Vec<_>>>()
        .expect("log lines");
    assert_eq!(log_content, vec!["ok"]);
}

example!(t_100_args, "100-args");
example!(t_101_atime, "101-atime");
example!(t_102_empty, "102-empty");
example!(t_103_unicode, "103-unicode");
example!(t_104_space, "104-space");

#[test]
fn t_110_compile() {
    let tc = TestCase::new("110-compile").expect("setup");
    tc.run().expect("110-compile");

    let hello = tc.example_dir().join("hello");

    println!("Test case dir: {:?}", tc);

    let _ = fs::metadata(&hello)
        .chain_err(|| format!("Built hello at {:?}", hello))
        .expect("hello");
    let out = Command::new(&hello).output().expect("spawn hello");
    assert!(
        out.status.success(),
        "Compiled hello ({:?}) ran okay",
        hello
    );
}

example!(t_111_compile2, "111-compile2");

example!(t_120_defaults_flat, "120-defaults-flat");
example!(t_121_defaults_nested, "121-defaults-nested",
    #[should_panic]);
// example!(t_130_mode, "130-mode");
example!(t_140_shuffle, "140-shuffle");
example!(t_141_keep_going, "141-keep-going");
// example!(t_200_shell, "200-shell");
example!(t_201_fail, "201-fail");
// example!(t_202_del, "202-del");
example!(t_220_ifcreate, "220-ifcreate");
// example!(t_250_makedir, "250-makedir");
example!(t_350_deps, "350-deps");
example!(t_550_chdir, "550-chdir");
// example!(t_640_always, "640-always");
// example!(t_660_stamp, "660-stamp");
example!(t_950_curse, "950-curse");
// example!(t_999_installer, "999-installer");
