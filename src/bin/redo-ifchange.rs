use std::env;
use std::process::Command;
use std::os::unix::process::CommandExt;

fn main() {
    let mut cmd = Command::new("redonk");
    cmd.arg("redoifchange");
    cmd.args(env::args().skip(1));

    let err = cmd.exec();
    assert!(false, "exec failed: {:?}", err);
}
