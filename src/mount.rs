use fuser::{Config, Filesystem, MountOption};

struct Pelican;

impl Filesystem for Pelican {}

pub fn mount(_image: &str, mountpoint: &str) {
    let mut config = Config::default();
    config.mount_options = vec![MountOption::RO, MountOption::FSName("pelican".to_string())];

    fuser::mount(Pelican, mountpoint, &config).unwrap();
}
