#![allow(dead_code)]
#[path = "../src/domain.rs"]
mod domain;
use domain::Location;
use std::ffi::OsStr;
fn main() {
    let prefix = Location::S3 {
        connection: "fixture".into(),
        bucket: "fixture".into(),
        key: "a//../".into(),
        prefix: true,
    };
    let child = prefix.join(OsStr::new("file.txt")).unwrap();
    assert!(matches!(&child,Location::S3{key,..} if key=="a//../file.txt"));
    assert_eq!(child.parent(), Some(prefix.clone()));
    assert!(prefix.join(OsStr::new("../outside")).is_err());
    let sftp = Location::Sftp {
        connection: "fixture".into(),
        path: "/".into(),
    };
    assert!(sftp.parent().is_none());
    assert_eq!(
        sftp.join(OsStr::new("file")).unwrap().display(),
        "sftp://fixture/file"
    );
    assert!(sftp.parse_path("relative").is_err());
    assert_eq!(sftp.parse_path(&sftp.display()).unwrap(), sftp);
    let local = Location::Local("/tmp/file".into());
    assert_eq!(local.parent(), Some(Location::Local("/tmp".into())));
    println!("Provider-qualified locations and exact S3 key semantics passed");
}
