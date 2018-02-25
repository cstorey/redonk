#[macro_use]
extern crate structopt;
#[macro_use]
extern crate clap;
#[macro_use]
extern crate error_chain;
#[macro_use]
extern crate serde_derive;
extern crate serde;
extern crate serde_json;

use std::path::{Path,PathBuf};
use std::fs;
use std::io;

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
enum Kind {
    Source,
    Target,
}
#[derive(Deserialize, Debug)]
struct Item {
    name: PathBuf,
    kind: Kind,
    uptodate: Option<bool>,
}
struct Store;

impl Item {

    fn new_target(path: &Path) -> Self {
        Item {
            name: path.to_owned(),
            kind: Kind::Target,
            uptodate: None,
        }
    }

    fn as_source(mut self) -> Self {
        self.kind = Kind::Source;
        self
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
        // let name = fs::canonicalize(name)?;
        let fname = name.file_name().and_then(|s| s.to_str()).expect("PathBuf::file_name");
        let state_fname = format!(".redonk.{}", fname);
        Ok(name.with_file_name(state_fname))
    }

    fn read(&self, name: &Path) -> Result<Option<Item>> {
        let state_file = self.state_file_of(name)?;
        let readerp = fs::File::open(&state_file)
            .map(Some)
            .or_else(|e| if e.kind() == io::ErrorKind::NotFound { Ok(None) } else { Err(e) })?;
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

fn redo_ifchange(store: &mut Store, targets: &[PathBuf]) -> Result<()> {
    // Start off just by rebuilding, like, everything.
    for target in targets {
        let it = store.read(&target)?
            .map(|it| it.as_source())
            .unwrap_or_else(|| Item::new_target(&target));
        
        if it.uptodate.is_some() {
            unimplemented!("record i as a regular prerequiste for its parent");
            continue
        }
    }


    Ok(())
}

fn main() {
    let Opt { op, targets } = Opt::from_args();
    println!("op: {:?}; targets: {:?}", op, targets);
    let targets = targets.into_iter().map(PathBuf::from).collect::<Vec<_>>();

    let mut store = Store::new().expect("Store::new");
    match op {
        Operation::Redo => redo(&mut store, &targets).expect("redo"),
        Operation::RedoIfChange => redo_ifchange(&mut store, &targets).expect("redo-ifchange"),
        other => unimplemented!("{:?}", other),
    }
}


// fn main() { panic!() }
