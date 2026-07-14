use std::io;
use std::path::Path;

#[cfg(windows)]
pub type LocalListener = std::net::TcpListener;
#[cfg(windows)]
pub type LocalStream = std::net::TcpStream;

#[cfg(unix)]
pub type LocalListener = std::os::unix::net::UnixListener;
#[cfg(unix)]
pub type LocalStream = std::os::unix::net::UnixStream;

#[cfg(unix)]
pub fn bind(endpoint: &Path) -> io::Result<LocalListener> {
    if endpoint.exists() {
        let _ = std::fs::remove_file(endpoint);
    }
    LocalListener::bind(endpoint)
}

#[cfg(unix)]
pub fn connect(endpoint: &Path) -> io::Result<LocalStream> {
    LocalStream::connect(endpoint)
}

#[cfg(windows)]
pub fn bind(endpoint: &Path) -> io::Result<LocalListener> {
    let listener = LocalListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let address = listener.local_addr()?;
    let temporary = endpoint.with_extension("endpoint.tmp");
    std::fs::write(&temporary, address.to_string())?;
    if endpoint.exists() {
        std::fs::remove_file(endpoint)?;
    }
    std::fs::rename(temporary, endpoint)?;
    Ok(listener)
}

#[cfg(windows)]
pub fn connect(endpoint: &Path) -> io::Result<LocalStream> {
    let address = std::fs::read_to_string(endpoint)?;
    LocalStream::connect(address.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn local_transport_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let endpoint = temp.path().join("archcar-test.endpoint");
        let listener = bind(&endpoint).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0_u8; 4];
            stream.read_exact(&mut bytes).unwrap();
            assert_eq!(&bytes, b"ping");
            stream.write_all(b"pong").unwrap();
        });

        let mut client = connect(&endpoint).unwrap();
        client.write_all(b"ping").unwrap();
        let mut bytes = [0_u8; 4];
        client.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"pong");
        server.join().unwrap();
    }
}
