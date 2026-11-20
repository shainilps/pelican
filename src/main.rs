use std::env;

mod inode;
mod mkfs;
mod mount;
mod util;

enum Command {
    Mkfs { image: String },
    Mount { image: String, mountpoint: String },
}

const USAGE: &str = "
  pelican: A Time-Machine File-System in userspace (FUSE)

  usage:
    pelican <command> <args...>

  commands:
    mkfs    <source>
      - used to format the source or disk equivalent

    mount   <source> <directory>
      -  actual command to use this  program duhhh!!
  ";

fn parse_args() -> Result<Command, String> {
    let args: Vec<String> = env::args().skip(1).collect();

    match args.as_slice() {
        [] => Err(USAGE.to_string()),

        [cmd, rest @ ..] => match (cmd.as_str(), rest) {
            ("mkfs", [image]) => Ok(Command::Mkfs {
                image: image.clone(),
            }),
            ("mount", [image, mountpoint]) => Ok(Command::Mount {
                image: image.clone(),
                mountpoint: mountpoint.clone(),
            }),

            ("mkfs", _) => Err(USAGE.to_string()),
            ("mount", _) => Err(USAGE.to_string()),

            (_, _) => Err(USAGE.to_string()),
        },
    }
}

fn main() {
    env_logger::init();

    let command = match parse_args() {
        Ok(command) => command,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };

    match command {
        Command::Mkfs { image } => {
            if let Err(e) = mkfs::mkfs(&image) {
                eprintln!("mkfs: {e}");
                std::process::exit(1);
            }
        }
        Command::Mount { image, mountpoint } => {
            mount::mount(&image, &mountpoint);
        }
    }
}
