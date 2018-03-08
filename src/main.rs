#![cfg_attr(all(test, feature = "impl_trait"), feature(conservative_impl_trait))]

#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate error_chain;
extern crate fs2;
#[macro_use]
extern crate log;
extern crate rand;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate structopt;

#[cfg(all(test, feature = "impl_trait"))]
extern crate suppositions;
#[cfg(all(test, feature = "impl_trait"))]
extern crate tempdir;

use std::path::{Component, Path, PathBuf};
use std::process::{self, Command};
use std::fs;
use std::io;
use std::env;
use std::ffi::{OsStr, OsString};
use std::collections::VecDeque;
use std::os::linux::fs::MetadataExt;
use fs2::FileExt;

use structopt::StructOpt;

const EXIT_SUCCESS: i32 = 0;
const EXIT_FAILURE: i32 = 1;

error_chain! {
    foreign_links {
        Io(::std::io::Error);
        Json(serde_json::Error);
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

fn optionally_exists<T>(
    r: ::std::result::Result<T, io::Error>,
) -> ::std::result::Result<Option<T>, io::Error> {
    r.map(Some).or_else(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            Ok(None)
        } else {
            Err(e)
        }
    })
}

fn exists(path: &Path) -> Result<bool> {
    let maybe_stat = optionally_exists(fs::metadata(&path))?;
    Ok(maybe_stat.is_some())
}

impl Item {
    fn new_target(path: &Path) -> Self {
        Item {
            name: path.to_owned(),
            uptodate: None,
        }
    }

    fn find_builder(&self) -> Result<Builder> {
        let cwd = Path::new(".").canonicalize()?;

        let mut path = cwd.join(&self.name);
        let fname = path.file_name()
            .chain_err(|| format!("Builder file name for {:?}", self))?
            .to_str()
            .chain_err(|| format!("Could not decode filename as utf-8: {:?}", path))?
            .to_owned();

        while path.pop() {
            if let Some(builder) = self.search_target_in_dir(&fname, &path)? {
                return Ok(builder);
            }
        }
        return Err(format!("Could not find builder for {:?}", self).into());
    }

    fn search_target_in_dir(&self, fname: &str, dir: &Path) -> Result<Option<Builder>> {
        for suffix in FileSuffixTails::new(&fname) {
            let is_default = suffix.is_empty() || suffix.chars().next() == Some('.');
            let name = format!("{}{}.do", if is_default { "default" } else { "" }, suffix);

            let candidate = dir.join(name);
            debug!("Considering path: {:?}", candidate);

            if exists(&candidate)? {
                return Ok(Some(Builder::new(&candidate, is_default)?));
            };
        }

        Ok(None)
    }

    fn is_target(&self) -> Result<bool> {
        let res = !exists(&self.name)?;
        debug!("is_target: {:?} → {:?}", self, res);
        Ok(res)
    }
    fn redo(&self, xtrace: bool) -> Result<()> {
        if self.is_target()? {
            info!("Target: {:?}", self);
            let dofile = self.find_builder()?;
            debug!(
                "Build: {:?} with {:?} in {:?}",
                self,
                dofile,
                env::current_dir()
            );

            dofile.perform(&self, xtrace).chain_err(|| "perform")?;
        } else {
            debug!("Presumed source file: {:?}", self);
        }

        Ok(())
    }

    fn dir_path(&self) -> Result<PathBuf> {
        let dir = self.name
            .parent()
            .chain_err(|| format!("Target: {:?} missing parent", self))?;
        Ok(dot_if_empty(dir).to_owned())
    }

    fn file_name(&self) -> Result<OsString> {
        let file_name = self.name
            .file_name()
            .chain_err(|| format!("Target: {:?} missing filename", self))?;
        Ok(file_name.to_owned())
    }

    fn path(&self) -> &Path {
        &self.name
    }

    fn abs_path(&self) -> Result<PathBuf> {
        let target_dir = self.dir_path()?;
        let target_dir_abs = target_dir
            .canonicalize()
            .chain_err(|| format!("canonicalize target dir {:?}", target_dir))?;
        let target_filename = self.file_name()?;
        Ok(target_dir_abs.join(target_filename))
    }

    fn tempfile(&self) -> Result<TempFile> {
        TempFile::sibling_of(&self.abs_path()?)
    }
}

#[derive(Debug)]
struct TempFile {
    path: PathBuf,
    file: Option<fs::File>,
}

impl TempFile {
    fn sibling_of(target: &Path) -> Result<TempFile> {
        let mut path = target.to_owned();

        let tmpf_lock = target
            .parent()
            .chain_err(|| format!("Target with no filename? {:?}", target))?
            .join(".lock");

        let lock = fs::File::create(&tmpf_lock)?;
        lock.lock_exclusive()?;

        loop {
            path.set_file_name(format!(".tmpf-redonk-{:x}", rand::random::<u64>()));
            if !exists(&path)? {
                let tmpf = fs::File::create(&path)?;
                lock.unlock()?;
                return Ok(TempFile {
                    file: Some(tmpf),
                    path: path.to_owned(),
                });
            }
        }
    }
}

#[derive(Debug)]
struct Builder {
    dofile: PathBuf,
    default: bool,
}

fn dot_if_empty(p: &Path) -> &Path {
    if p.as_os_str().is_empty() {
        Path::new(".")
    } else {
        p
    }
}

impl Builder {
    fn new(dofile: &Path, default: bool) -> Result<Builder> {
        let dofile = dofile
            .canonicalize()
            .chain_err(|| format!("Canonicalize in Builder::new({:?}, {:?})", dofile, default))?
            .to_owned();
        Ok(Builder { dofile, default })
    }

    fn base_of<'a>(&self, target_name: &'a Path) -> Result<&'a OsStr> {
        let target_name_s = target_name
            .to_str()
            .chain_err(|| format!("Target file {:?} not utf-8 encoded?", &target_name))?;
        let target_fname = target_name
            .file_name()
            .chain_err(|| format!("Target {:?} has no file name?", &target_name))?
            .to_str()
            .chain_err(|| format!("Target file {:?} not utf-8 encoded?", &target_name))?;
        let pattern = self.dofile
            .file_name()
            .chain_err(|| format!("Build file {:?} has no file name?", &target_name))?
            .to_str()
            .chain_err(|| format!("Build file {:?} not utf-8 encoded?", &target_name))?;

        let default_prefix = "default";
        let do_suffix = ".do";
        let target_base = if pattern.starts_with(default_prefix) {
            let p_tail = &pattern[default_prefix.len()..pattern.len() - do_suffix.len()];

            let base_end = target_name_s.len() - p_tail.len();
            let t_tail = &target_name_s[base_end..];

            // Clearly, I've missed a way to not have to re-derive this.
            // Maybe figure this out when scanning for build files?
            assert_eq!(
                p_tail,
                t_tail,
                "Pattern tail {:?} (from {:?}) should equal target tail: {:?} (from {:?})",
                p_tail,
                pattern,
                t_tail,
                target_fname
            );
            &target_name_s[..base_end]
        } else {
            target_name_s
        };

        debug!(
            "Builder::base_of({:?}, {:?}) → {:?}",
            self, target_name, target_base
        );
        Ok(OsStr::new(target_base))
    }

    fn perform(&self, target: &Item, xtrace: bool) -> Result<()> {
        let target_abs = target.abs_path()?;

        let mut stdout_temp = target.tempfile()?;
        let mut named_temp = target.tempfile()?;
        debug!(
            "Target : {:?}",
            target_abs /* .components().collect::<Vec<_>>()*/
        );

        let mut cmd = self.build_command(&target_abs, &mut stdout_temp, &mut named_temp, xtrace)?;
        debug!("⇒ {:?} ({:?})", self.dofile, cmd);
        let res = cmd.spawn()?.wait()?;
        debug!("⇐ {:?}", self.dofile);

        if !res.success() {
            return Err(format!(
                "Dofile: {:?} exited with code:{:?}",
                self.dofile,
                res.code()
            ).into());
        }

        let stdout_size = fs::metadata(&stdout_temp.path)?.len();
        // it's fine if someone wants to delete $3.
        let named_size = optionally_exists(fs::metadata(&named_temp.path))?
            .map(|s| s.len())
            .unwrap_or(0);

        debug!("stdout: {:?} size:{:?}", &stdout_temp.path, stdout_size);
        debug!("named: {:?} size:{:?}", &named_temp.path, named_size);

        match (stdout_size, named_size) {
            (0, 0) | (_, 0) => {
                debug!("{:?} → {:?}", &stdout_temp.path, target);
                fs::rename(stdout_temp.path, target.path())
                    .chain_err(|| "Persist stdout tempfile")?;
            }
            (0, _) => {
                debug!("{:?} → {:?}", &named_temp.path, target);
                fs::rename(named_temp.path, target.path()).chain_err(|| "Persist stdout tempfile")?;
            }
            (_, _) => {
                panic!("Both $3 and stdout written to!");
            }
        }

        Ok(())
    }

    fn build_command(
        &self,
        target_abs: &Path,
        stdout: &mut TempFile,
        named_temp: &mut TempFile,
        xtrace: bool,
    ) -> Result<Command> {
        let builder_abs = self.dofile.canonicalize()?;

        debug!(
            "Builder: {:?}",
            builder_abs /*.components().collect::<Vec<_>>()  */
        );

        let target_dir = target_abs.parent().unwrap_or(Path::new("."));
        let builder_dir = builder_abs
            .parent()
            .chain_err(|| format!("Builder path {:?} has no parent", builder_abs))?;
        let target_name = target_abs.relative_to_dir(&builder_dir);
        warn!(
            "{:?} relative_to_dir {:?} => {:?}",
            target_abs, builder_dir, target_name
        );
        let target_base = if self.default {
            self.base_of(&target_name)?
        } else {
            target_name.as_ref()
        };

        debug!(
            "target_name: {:?}; base: {:?}; cwd: {:?}",
            target_name, target_base, target_dir
        );
        let mut cmd = if self.is_executable()? {
            Command::new(&self.dofile)
        } else {
            let mut cmd = Command::new("sh");
            cmd.arg("-e");
            if xtrace {
                cmd.arg("-x");
            };
            cmd.arg(&self.dofile);
            cmd
        };

        cmd
            // $1: Target name
            .arg(&target_name)
            // $2: Basename of the target
            .arg(&target_base)
            // $3: temporary output file.
            .arg(named_temp.path.relative_to_dir(&builder_dir));
        cmd.current_dir(builder_dir);

        cmd.stdout(stdout.file.take().expect("take stdout temp file"));

        // Emulate apenwarr's minimal/do
        cmd.env("DO_BUILT", "t");

        Ok(cmd)
    }

    fn is_executable(&self) -> Result<bool> {
        let stat = fs::metadata(&self.dofile)?;
        let mode_bits = stat.st_mode();

        // For now, assume that if _any_ are set, then it's meant to be executed.
        // the alternative is to try `execve` and fall back otherwise, AFAICS.
        Ok((mode_bits & 0o0111) != 0)
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
        if let Some(r) = optionally_exists(fs::File::open(&state_file))? {
            let res = serde_json::from_reader(r)?;
            Ok(Some(res))
        } else {
            Ok(None)
        }
    }
}

fn redo(store: &mut Store, targets: &[PathBuf], xtrace: bool) -> Result<()> {
    // Mark targets as non-up to date
    redo_ifchange(store, targets, xtrace)
}

// Sack off the main algorithm bits for now; just implement the minimal redo
// version. Ie: Rebuild everything. Avoid loops by `.did` files.
// If a file exists and can't find a `.do` rule, assume it is source.
//
// Then extend with redo on mtime change, and redo on mtime+content change.
//
fn redo_ifchange(store: &mut Store, targets: &[PathBuf], xtrace: bool) -> Result<()> {
    // Start off just by rebuilding, like, everything.
    for target in targets {
        let it = store
            .read(&target)?
            .unwrap_or_else(|| Item::new_target(&target));

        it.redo(xtrace)?;
    }

    Ok(())
}

fn redo_ifcreate(_store: &mut Store, targets: &[PathBuf], _xtrace: bool) -> Result<()> {
    debug!("redo-ifcreate {:?} ignored", targets);
    Ok(())
}

fn main() {
    env_logger::init();

    debug!("✭: {:?}", env::args().collect::<Vec<_>>());
    let Opt { op, targets } = Opt::from_args();

    let xtrace = env::var_os("REDONK_XTRACE").is_some();

    let code = match run(op, &targets, xtrace) {
        Ok(_) => EXIT_SUCCESS,
        Err(e) => {
            eprintln!("Could not build targets: {:?}\n{:?}", targets, e);
            EXIT_FAILURE
        }
    };
    process::exit(code);
}

fn run(op: Operation, targets: &[String], xtrace: bool) -> Result<()> {
    debug!(
        "op: {:?}; targets: {:?}; in:{:?}",
        op,
        targets,
        env::current_dir()
    );
    let targets = targets.into_iter().map(PathBuf::from).collect::<Vec<_>>();

    let mut store = Store::new().expect("Store::new");
    match op {
        Operation::Redo => redo(&mut store, &targets, xtrace).chain_err(|| "redo"),
        Operation::RedoIfChange => {
            redo_ifchange(&mut store, &targets, xtrace).chain_err(|| "redo-ifchange")
        }
        Operation::RedoIfCreate => {
            redo_ifcreate(&mut store, &targets, xtrace).chain_err(|| "redo-ifcreate")
        }
    }
}

// fn main() { panic!() }

trait PathExt {
    // This is used to figure out what path a target has relative to a _directory_.
    fn relative_to_dir<P: AsRef<Path>>(&self, base: P) -> PathBuf;
}

impl<P: AsRef<Path>> PathExt for P {
    fn relative_to_dir<P2: AsRef<Path>>(&self, base: P2) -> PathBuf {
        trace!("{:?} relative_to_dir: {:?}", self.as_ref(), base.as_ref());
        assert!(
            self.as_ref().is_absolute(),
            "subject path {:?} not absolute",
            self.as_ref()
        );
        assert!(
            base.as_ref().is_absolute(),
            "base path {:?} not absolute",
            base.as_ref()
        );
        let mut subject = self.as_ref().components().peekable();
        let mut base_rf = base.as_ref().components().peekable();
        let mut popped = VecDeque::new();

        while subject
            .peek()
            .and_then(|b| base_rf.peek().map(|r| b == r))
            .unwrap_or(false)
        {
            let subj = subject.next();
            let _base = base_rf.next();

            trace!("Dicard: subj: {:?}; t: {:?}", subj, _base);
            popped.push_back(subj);
        }

        let remaining_subject = subject.map(|c| c.as_os_str()).collect::<PathBuf>();
        let remaining_base = base_rf.clone().map(|c| c.as_os_str()).collect::<PathBuf>();
        trace!(
            "remaining: subject: {:?}; base: {:?}",
            remaining_subject,
            remaining_base
        );

        let mut prefix = PathBuf::new();
        for component in base_rf {
            trace!("Component: {:?}", component);
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    if !prefix.pop() {
                        unimplemented!("Pop start of prefix; common: {:?}", popped)
                    }
                }
                _ => prefix.push(".."),
            };
        }

        trace!("Prefix: {:?}; subj: {:?}", prefix, remaining_subject);
        return prefix.join(remaining_subject);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn file_suffix_tails_should_return_pathname_tails() {
        let cs = FileSuffixTails::new("foo.bar.baz");
        let options = vec!["foo.bar.baz", ".bar.baz", ".baz", ""];

        assert_eq!(cs.collect::<Vec<_>>(), options);
    }

    #[test]
    fn path_relativize_should_handle_items_in_same_directory() {
        assert_eq!(
            Path::new("/hello/world").relative_to_dir(&Path::new("/hello")),
            Path::new("world")
        );
    }

    #[test]
    fn path_relativize_should_handle_subject_in_child_directory() {
        assert_eq!(
            Path::new("/hello/world").relative_to_dir(&Path::new("/.")),
            Path::new("hello/world")
        );
    }

    #[test]
    fn path_relativize_should_handle_base_in_child_directory() {
        assert_eq!(
            Path::new("/hello").relative_to_dir(&Path::new("/world")),
            Path::new("../hello")
        );
    }

    #[test]
    fn path_relativize_should_handle_base_in_child_directory_trailing_slash() {
        assert_eq!(
            Path::new("/hello").relative_to_dir(&Path::new("/world/")),
            Path::new("../hello")
        );
    }

    #[test]
    fn path_relativize_should_handle_items_in_same_directory_with_common_prefix() {
        assert_eq!(
            Path::new("/a/hello/world").relative_to_dir(&Path::new("/a/hello")),
            Path::new("world")
        );
    }

    #[test]
    fn path_relativize_should_handle_subject_in_child_directory_with_common_prefix() {
        assert_eq!(
            Path::new("/a/hello/world").relative_to_dir(&Path::new("/a/")),
            Path::new("hello/world")
        );
    }

    #[test]
    fn path_relativize_should_handle_base_in_child_directory_with_common_prefix() {
        assert_eq!(
            Path::new("/the/hello").relative_to_dir(&Path::new("/the/world")),
            Path::new("../hello")
        );
    }
}

#[cfg(all(test, feature = "impl_trait"))]
mod model_tests {
    use suppositions::*;
    use suppositions::data::DataError;
    use suppositions::generators::*;
    use tempdir::TempDir;
    use std::path::*;
    use std::fs;
    use super::*;

    fn paths() -> impl Generator<Item = PathBuf> {
        let component = one_of(consts("."))
            .or(consts(".."))
            .or(consts("foo"))
            .or(consts("bar"))
            .or(consts("baz"))
            .or(consts("quux"))
            .or(consts("quuux"));

        vecs(component).map(|cs| cs.into_iter().collect::<PathBuf>())
            // this part canonicalises the representation, discarding 
            // trailing "/."s and the like.
            .filter(|p| p.as_os_str().len() > 0)
            .filter(|p| if let Some(Component::Normal(_)) = p.components().last()  { true } else { false } )
            .map(|p| p.components().collect::<PathBuf>())
    }

    fn component_movement(depth: &mut isize, c: Component) -> Option<isize> {
        let delta = match c {
            Component::CurDir => 0,
            Component::ParentDir => -1,
            Component::Normal(_) => 1,
            other => unimplemented!("cannot yet handle: {:?}", other),
        };

        *depth += delta;
        Some(*depth)
    }

    fn mkpath(tmpd: &TempDir, path: &Path) -> ::std::result::Result<(), io::Error> {
        let path = tmpd.path().join(path);
        if let Some(p) = path.parent() {
            trace!("Create dir: {:?}", p);
            fs::create_dir_all(p)?;
        };

        trace!("Create file: {:?}", path);
        let _ = fs::File::create(&path)?;
        Ok(())
    }
    fn mkpaths(base: &Path, target: &Path) -> ::std::result::Result<TempDir, io::Error> {
        let tmpd = TempDir::new("should_behave_as_filesystem_traversal").expect("tempdir");
        for f in [&base, &target].iter() {
            mkpath(&tmpd, f)?
        }
        Ok(tmpd)
    }

    #[test]
    #[ignore]
    fn should_behave_as_filesystem_traversal() {
        let gen = (paths(), paths())
            .filter(|&(ref base, ref target)| {
                // Assert base is not a prefix of the target, and vica versa
                return base.strip_prefix(&target).is_err() && target.strip_prefix(&base).is_err();
            })
            .filter(|&(ref base, ref target)| {
                // Neither base nor target should ascend beyond their "root" for now.
                let b = base.components().scan(0, component_movement).all(|d| d > 0);
                let t = target
                    .components()
                    .scan(0, component_movement)
                    .all(|d| d > 0);

                b && t
            })
            .filter_map(|(base, target): (PathBuf, PathBuf)| {
                println!("-- base: {:?}; target: {:?}", base, target);
                match mkpaths(&base, &target) {
                    Err(ref e) => {
                        println!("E: {:?}; kind: {:?}", e, e.kind());
                        return Err(DataError::SkipItem);
                    }
                    Ok(tmpd) => return Ok((tmpd, base, target)),
                }
            });

        property(gen).check(|(tmpd, base, target)| {
            let base_dir = base.parent().unwrap_or(Path::new("."));

            let relpath = target.relative_to_dir(&base_dir);
            println!("Relpath: {:?} / {:?}", relpath, tmpd.path().join(&relpath));
            let base_dir_canon = tmpd.path()
                .join(&base_dir)
                .canonicalize()
                .expect("canonicalize tmpd + base dir");
            println!(
                "Base: {:?} (dir {:?}); canonical: {:?}",
                base, base_dir, base_dir_canon
            );
            let targ_canon = tmpd.path()
                .join(&target)
                .canonicalize()
                .expect("canonicalize target");
            println!("Target: {:?}; canonical: {:?}", target, targ_canon);
            println!(
                "Relpath joined to base: {:?}",
                base_dir_canon.join(&relpath)
            );
            let rel_canon = base_dir_canon
                .join(&relpath)
                .canonicalize()
                .expect("canonicalize tmpd + relpath");
            println!("Relpath canonical: {:?}", rel_canon);

            assert_eq!(targ_canon, rel_canon,
                        "target {:?} (canonical {:?}) base {:?} (dir: {:?}; canon: {:?}) => relpath: {:?} canon: {:?}", 
                            target, targ_canon,
                            base, base_dir, base_dir_canon,
                            relpath, rel_canon);

            println!()
        })
    }
}
