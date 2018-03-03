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

#[derive(Debug)]
struct FileSuffixTails<'a> {
    input: Option<&'a str>,
    next_idx: Option<usize>,
}

impl<'a> FileSuffixTails<'a> {
    fn new(s: &'a str) -> FileSuffixTails<'a> {
        FileSuffixTails {
            input: Some(s),
            next_idx: Some(0),
        }
    }
}

impl<'a> Iterator for FileSuffixTails<'a> {
    type Item = &'a str;
    fn next(&mut self) -> Option<Self::Item> {
        trace!("Next: {:?}", self);
        match self {
            &mut FileSuffixTails {
                input: Some(input),
                next_idx: Some(i),
            } => {
                let current = &input[i..];
                let suffix = &input[i + 1..];

                self.input = Some(&suffix);
                self.next_idx = suffix.find('.');

                trace!("Done: {:?}", self);
                Some(current)
            }
            &mut FileSuffixTails {
                input: Some(_),
                next_idx: None,
            } => {
                self.input = None;
                trace!("Gasp: {:?}", self);
                Some("")
            }
            _ => {
                trace!("Finished: {:?}", self);
                None
            }
        }
    }
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

    fn find_builder(&self) -> Result<Builder> {
        let mut path = PathBuf::from(&self.name);
        let fname = path.file_name()
            .chain_err(|| format!("Builder file name for {:?}", self))?
            .to_str()
            .chain_err(|| format!("Could not decode filename as utf-8: {:?}", path))?
            .to_owned();

        for candidate in FileSuffixTails::new(&fname) {
            let is_default = candidate.is_empty() || candidate.chars().next() == Some('.');
            let name = format!(
                "{}{}.do",
                if is_default { "default" } else { "" },
                candidate
            );

            path.set_file_name(name);
            debug!("Considering path: {:?}", path);

            if exists(&path)? {
                return Ok(Builder::new(&path, is_default)?);
            };
        }
        return Err(format!("Could not find builder for {:?}", self).into());
    }

    fn is_target(&self) -> Result<bool> {
        let res = !exists(&self.name)?;
        debug!("is_target: {:?} → {:?}", self, res);
        Ok(res)
    }
    fn redo(&self) -> Result<()> {
        if self.is_target()? {
            debug!("Target: {:?}", self);
            let dofile = self.find_builder()?;
            debug!(
                "Build: {:?} with {:?} in {:?}",
                self,
                dofile,
                env::current_dir()
            );

            dofile.perform(&self.name).chain_err(|| "perform")?;
        } else {
            debug!("Presumed source file: {:?}", self);
        }

        Ok(())
    }
}

#[derive(Debug)]
struct Builder {
    dofile: PathBuf,
    default: bool,
}

impl Builder {
    fn new(dofile: &Path, default: bool) -> Result<Builder> {
        let dofile = dofile
            .canonicalize()
            .chain_err(|| format!("Canonicalize in Builder::new({:?}, {:?})", dofile, default))?
            .to_owned();
        Ok(Builder { dofile, default })
    }

    fn perform(&self, target: &Path) -> Result<()> {
        let tmpf = {
            let fname: &Path = target.as_ref();
            let parent = fname
                .parent()
                .into_iter()
                .filter(|s| !s.as_os_str().is_empty())
                .next()
                .unwrap_or(Path::new("."));
            let dir = parent
                .canonicalize()
                .chain_err(|| format!("perform: canon parent: {:?}", parent))?;
            tempfile::NamedTempFile::new_in(dir)?
        };

        debug!(
            "target path cmoponents: {:?}",
            target.components().collect::<Vec<_>>()
        );

        let mut cmd = Command::new("sh");
        {
            let target = Path::new(".").join(target);
            let dir = target.parent()
                // .filter(|p| !p.is_empty())
                .unwrap_or(Path::new("."));
            let target_name = target.file_name().chain_err(|| "Target has no file name?")?;
            let target_stem = if self.default {
                Path::new(target_name)
                    .file_stem()
                    .chain_err(|| format!("{:?} has no file stem", &target))?
            } else {
                target_name.as_ref()
            };

            debug!(
                "target_name: {:?}; stem: {:?}; cwd: {:?}",
                target_name, target_stem, dir
            );
            cmd.arg("-e")
                .arg("-x")
                .arg(&self.dofile)
                // $1: Target name
                .arg(target_name)
                // $2: Basename of the target
                .arg(&target_stem)
                // $3: temporary output file.
                .arg(tmpf.path());
            cmd.current_dir(dir);
        }

        cmd.stdout(tmpf.reopen()?);

        // Emulate apenwarr's minimal/do
        cmd.env("DO_BUILT", "t");

        debug!("⇒ {:?} ({:?})", self.dofile, cmd);
        let res = cmd.spawn()?.wait()?;
        debug!("⇐ {:?}", self.dofile);

        assert!(
            res.success(),
            "Dofile: {:?} exited with code:{:?}",
            self.dofile,
            res.code()
        );

        debug!("{:?} → {:?}", tmpf.path(), target);
        fs::rename(tmpf.path(), &target).chain_err(|| "Persist output tempfile")?;

        Ok(())
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

        it.redo()?;
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
    debug!(
        "op: {:?}; targets: {:?}; in:{:?}",
        op,
        targets,
        env::current_dir()
    );
    let targets = targets.into_iter().map(PathBuf::from).collect::<Vec<_>>();

    let mut store = Store::new().expect("Store::new");
    match op {
        Operation::Redo => redo(&mut store, &targets).expect("redo"),
        Operation::RedoIfChange => redo_ifchange(&mut store, &targets).expect("redo-ifchange"),
        Operation::RedoIfCreate => redo_ifcreate(&mut store, &targets).expect("redo-ifcreate"),
    }
}

// fn main() { panic!() }

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn file_suffix_tails_should_return_pathname_tails() {
        let cs = FileSuffixTails::new("foo.bar.baz");
        let options = vec!["foo.bar.baz", ".bar.baz", ".baz", ""];

        assert_eq!(cs.collect::<Vec<_>>(), options);
    }
}
