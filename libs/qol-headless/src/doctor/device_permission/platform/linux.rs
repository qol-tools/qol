use crate::doctor::device_permission::I2cProbe;
use std::fs;
use std::io;
use std::path::PathBuf;

pub struct LinuxI2cProbe;

impl I2cProbe for LinuxI2cProbe {
    fn probe(&self) -> io::Result<()> {
        let nodes = i2c_device_nodes()?;
        if nodes.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no /dev/i2c-* device nodes",
            ));
        }
        for node in nodes {
            fs::OpenOptions::new().read(true).write(true).open(node)?;
        }
        Ok(())
    }
}

fn i2c_device_nodes() -> io::Result<Vec<PathBuf>> {
    let mut nodes = Vec::new();
    for entry in fs::read_dir("/dev")? {
        let name = entry?.file_name();
        let name = name.to_string_lossy();
        if let Some(index) = name.strip_prefix("i2c-") {
            if !index.is_empty() && index.chars().all(|character| character.is_ascii_digit()) {
                nodes.push(PathBuf::from("/dev").join(name.as_ref()));
            }
        }
    }
    Ok(nodes)
}
