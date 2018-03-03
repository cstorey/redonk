#[macro_use]
extern crate clap;
#[macro_use]
extern crate error_chain;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate structopt;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;
use std::io;
use std::env;
use std::ffi;

use structopt::StructOpt;

arg_enum! {
    #[derive(Debug)]
    enum Operation {
        Redo,
        RedoIfChange,
        RedoIfCreate
    }
}

#[derive(StructOpt, Debug)]
struct Opt {
    /// Important argument.
    #[structopt(raw(possible_values = "&Operation::variants()", case_insensitive = "true"))]
    op: Operation,
    targets: Vec<String>,
}

#[derive(Deserialize, Debug)]
struct Item {
    name: PathBuf,
    uptodate: Option<bool>,
}
struct Store;

fn exists(path: &Path) -> Result<bool> {
    let exists = fs::metadata(&path).map(|_| true).or_else(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            Ok(false)
        } else {
            Err(e)
        }
    })?;
    Ok(exists)
}
impl Item {
    fn new_target(path: &Path) -> Self {
        Item {
            name: path.to_owned(),
            uptodate: None,
        }
    }

    fn find_builder(&self) -> Result<PathBuf> {
        // Try target.ext.do
        let mut path = PathBuf::from(&self.name);
        let mut fname = path.file_name()
            .chain_err(|| format!("Builder file name for {:?}", self))?
            .to_os_string();
        fname.push(".do");
        path.set_file_name(fname);

        if exists(&path)? {
            return Ok(path);
        };

        // try default.ext.do
        let mut path = PathBuf::from(&self.name);
        // This may be wrong for compounded extensions like foo.tar.gz
        let mut fname = ffi::OsString::from("default");
        if let Some(ext) = path.extension() {
            fname.push(".");
            fname.push(ext);
        }
        fname.push(".do");
        path.set_file_name(fname);

        if exists(&path)? {
            return Ok(path);
        };

        return Err(format!("Could not find builder for {:?}", self).into());
    }
}
error_chain! {
    foreign_links {
        Io(::std::io::Error) #[cfg(unix)];
        Json(serde_json::Error) #[cfg(unix)];
    }
}

impl Store {
    fn new() -> Result<Self> {
        Ok(Store)
    }

    fn state_file_of(&self, name: &Path) -> Result<PathBuf> {
        let fname = name.file_name()
            .and_then(|s| s.to_str())
            .expect("PathBuf::file_name");
        let state_fname = format!(".redonk.{}", fname);
        Ok(name.with_file_name(state_fname))
    }

    fn read(&self, name: &Path) -> Result<Option<Item>> {
        let state_file = self.state_file_of(name)?;
        let readerp = fs::File::open(&state_file).map(Some).or_else(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(e)
            }
        })?;
        if let Some(r) = readerp {
            let res = serde_json::from_reader(r)?;
            Ok(Some(res))
        } else {
            Ok(None)
        }
    }
}

fn redo(store: &mut Store, targets: &[PathBuf]) -> Result<()> {
    // Mark targets as non-up to date
    redo_ifchange(store, targets)
}

// Sack off the main algorithm bits for now; just implement the minimal redo
// version. Ie: Rebuild everything. Avoid loops by `.did` files.
// If a file exists and can't find a `.do` rule, assume it is source.
//
// Then extend with redo on mtime change, and redo on mtime+content change.
//
fn redo_ifchange(store: &mut Store, targets: &[PathBuf]) -> Result<()> {
    // Start off just by rebuilding, like, everything.
    for target in targets {
        let it = store
            .read(&target)?
            .unwrap_or_else(|| Item::new_target(&target));

        eprintln!("Target: {:?}: {:?}", target, it);
        let dofile = it.find_builder()?;
        eprintln!(
            "Build: {:?} with {:?} in {:?}",
            target,
            dofile,
            env::current_dir()
        );

        let mut cmd = Command::new("sh");
        cmd.arg("-e").arg(&dofile);
        eprintln!("Invoking: {:?}", cmd);
        let res = cmd.spawn()?.wait()?;

        assert!(res.success(), "Dofile: {:?} exited with {:?}", dofile, res);
    }

    Ok(())
}

fn main() {
    let Opt { op, targets } = Opt::from_args();
    eprintln!("op: {:?}; targets: {:?}", op, targets);
    let targets = targets.into_iter().map(PathBuf::from).collect::<Vec<_>>();

    let mut store = Store::new().expect("Store::new");
    match op {
        Operation::Redo => redo(&mut store, &targets).expect("redo"),
        Operation::RedoIfChange => redo_ifchange(&mut store, &targets).expect("redo-ifchange"),
        other => unimplemented!("{:?}", other),
    }
}

// fn main() { panic!() }
