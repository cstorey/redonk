use std::env;
use std::process::Command;
use std::os::unix::process::CommandExt;

fn main() {
    eprintln!("✪: {:?}", env::args().collect::<Vec<_>>());
    let mut cmd = Command::new("redonk");
    cmd.arg("redo");
    cmd.args(env::args().skip(1));

    let err = cmd.exec();
    assert!(false, "exec failed: {:?}", err);
}
