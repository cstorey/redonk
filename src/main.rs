#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate error_chain;
#[macro_use]
extern crate log;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate structopt;
extern crate tempfile;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;
use std::io;
use std::env;
use std::ffi;

use structopt::StructOpt;

error_chain! {
    foreign_links {
        Io(::std::io::Error);
        Json(serde_json::Error);
        TempFile(tempfile::PersistError);
    }
}

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

    fn is_target(&self) -> Result<bool> {
        let res = !exists(&self.name)?;
        debug!("is_target: {:?} → {:?}", self, res);
        Ok(res)
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

        if it.is_target()? {
            debug!("Target: {:?}: {:?}", target, it);
            let dofile = it.find_builder()?;
            debug!(
                "Build: {:?} with {:?} in {:?}",
                target,
                dofile,
                env::current_dir()
            );

            let tmpf = {
                let fname: &Path = it.name.as_ref();
                let dir: &Path = fname.parent().unwrap_or(Path::new("."));
                tempfile::NamedTempFile::new_in(dir)?
            };


            let mut cmd = Command::new("sh");
            {
                let target_name = it.name.clone();
                let target_stem = it.name.file_stem().chain_err(|| format!("{:?} has no file stem", it))?;
                cmd.arg("-ex")
                    .arg(&dofile)
                    // $1: Target name
                    .arg(target_name)
                    // $2: Basename of the target
                    .arg(&target_stem)
                    // $3: temporary output file.
                    .arg(tmpf.path());
            }

            cmd.stdout(tmpf.reopen()?);

            debug!("⇒ {:?} ({:?})", dofile, cmd);
            let res = cmd.spawn()?.wait()?;
            debug!("⇐ {:?}", dofile);

            assert!(res.success(), "Dofile: {:?} exited with {:?}", dofile, res);

            debug!("{:?} → {:?}", tmpf.path(), it.name);
            fs::rename(tmpf.path(), it.name).chain_err(|| "Persist output tempfile")?
        }
    }

    Ok(())
}

fn redo_ifcreate(_store: &mut Store, targets: &[PathBuf]) -> Result<()> {
    debug!("redo-ifcreate {:?} ignored", targets);
    Ok(())
}

fn main() {
    env_logger::init();

    debug!("✭: {:?}", env::args().collect::<Vec<_>>());
    let Opt { op, targets } = Opt::from_args();
    debug!("op: {:?}; targets: {:?}", op, targets);
    let targets = targets.into_iter().map(PathBuf::from).collect::<Vec<_>>();

    let mut store = Store::new().expect("Store::new");
    match op {
        Operation::Redo => redo(&mut store, &targets).expect("redo"),
        Operation::RedoIfChange => redo_ifchange(&mut store, &targets).expect("redo-ifchange"),
        Operation::RedoIfCreate => redo_ifcreate(&mut store, &targets).expect("redo-ifcreate"),
    }
}

// fn main() { panic!() }
