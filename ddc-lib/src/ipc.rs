use anyhow::Context;
use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Write a length-prefixed JSON message: [u32 LE byte count][JSON bytes].
pub async fn write_message<W, T>(writer: &mut W, msg: &T) -> anyhow::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(msg).context("serialize message")?;
    let len = payload.len() as u32;
    writer
        .write_all(&len.to_le_bytes())
        .await
        .context("write length prefix")?;
    writer.write_all(&payload).await.context("write payload")?;
    writer.flush().await.context("flush")?;
    Ok(())
}

/// Read a length-prefixed JSON message.
pub async fn read_message<R, T>(reader: &mut R) -> anyhow::Result<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .context("read length prefix")?;
    let len = u32::from_le_bytes(len_buf) as usize;

    // Sanity cap: 1 MiB
    anyhow::ensure!(len <= 1024 * 1024, "message too large: {} bytes", len);

    let mut payload = vec![0u8; len];
    reader
        .read_exact(&mut payload)
        .await
        .context("read payload")?;

    serde_json::from_slice(&payload).context("deserialize message")
}

/// Default socket path.
///
/// When the daemon runs as root (system service) the socket lives at `/run/ddcd.sock`
/// so any user can connect to it.  Non-root daemon instances use `$XDG_RUNTIME_DIR`.
/// The client auto-detects: if `/run/ddcd.sock` exists it is preferred, otherwise
/// it falls back to the XDG path (for user-session daemons on other machines).
pub fn default_socket_path() -> std::path::PathBuf {
    let system_path = std::path::PathBuf::from("/run/ddcd.sock");
    let uid = unsafe { libc::getuid() };

    if uid == 0 {
        // Running as root — always use the system path.
        return system_path;
    }

    // Non-root: prefer the system socket if the daemon is already running there.
    if system_path.exists() {
        return system_path;
    }

    // Fall back to the user-session socket.
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        std::path::PathBuf::from(runtime_dir).join("ddcd.sock")
    } else {
        std::path::PathBuf::from(format!("/tmp/ddcd-{uid}.sock"))
    }
}
