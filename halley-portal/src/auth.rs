//! Authenticate the portal frontend, then pin its unique bus identity on objects.
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use zbus::names::{BusName, OwnedUniqueName};
use zbus::{blocking::Connection, fdo, message::Header};

pub fn frontend(connection: &Connection, header: &Header<'_>) -> fdo::Result<OwnedUniqueName> {
    let sender = header.sender().ok_or_else(denied)?;
    let bus = zbus::blocking::fdo::DBusProxy::new(connection)?;
    let owner = bus
        .get_name_owner(BusName::try_from("org.freedesktop.portal.Desktop").unwrap())
        .map_err(|_| denied())?;
    same_owner(sender.as_str(), owner.as_str())?;
    let name = BusName::Unique(sender.clone());
    let uid = bus
        .get_connection_unix_user(name.clone())
        .map_err(|_| denied())?;
    let pid = bus
        .get_connection_unix_process_id(name)
        .map_err(|_| denied())?;
    if uid != unsafe { libc::geteuid() } {
        return Err(denied());
    }
    let executable = std::fs::metadata(format!("/proc/{pid}/exe")).map_err(|_| denied())?;
    let approved = std::env::var_os("HALLEY_PORTAL_FRONTEND").map(std::path::PathBuf::from);
    let paths = approved.map(|path| vec![path]).unwrap_or_else(|| {
        vec![
            "/usr/lib/xdg-desktop-portal".into(),
            "/usr/libexec/xdg-desktop-portal".into(),
        ]
    });
    if !paths
        .iter()
        .filter(|path| path.is_absolute())
        .any(|path| matches_executable(&executable, path))
    {
        return Err(denied());
    }
    Ok(owner)
}

fn matches_executable(actual: &std::fs::Metadata, approved: &Path) -> bool {
    approved.is_absolute()
        && std::fs::metadata(approved).is_ok_and(|expected| {
            actual.is_file()
                && expected.is_file()
                && actual.dev() == expected.dev()
                && actual.ino() == expected.ino()
        })
}

pub fn same_owner(sender: &str, owner: &str) -> fdo::Result<()> {
    if sender.starts_with(':') && sender == owner {
        Ok(())
    } else {
        Err(denied())
    }
}

fn denied() -> fdo::Error {
    fdo::Error::AccessDenied(
        "only the authenticated xdg-desktop-portal frontend may access this object".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_missing_replaced_and_well_known_owners() {
        assert!(same_owner(":1.2", ":1.2").is_ok());
        assert!(same_owner(":1.3", ":1.2").is_err());
        assert!(same_owner("", "").is_err());
        assert!(
            same_owner(
                "org.freedesktop.portal.Desktop",
                "org.freedesktop.portal.Desktop"
            )
            .is_err()
        );
    }
    #[test]
    fn executable_grants_compare_file_identity_not_namespace_path_names() {
        let actual = std::fs::metadata("/proc/self/exe").unwrap();
        assert!(matches_executable(
            &actual,
            &std::env::current_exe().unwrap()
        ));
        assert!(!matches_executable(&actual, Path::new("/bin/sh")));
        assert!(!matches_executable(
            &actual,
            Path::new("xdg-desktop-portal")
        ));
    }

    #[test]
    fn an_untrusted_bus_client_cannot_gain_access_by_claiming_the_portal_name() {
        // Use a private bus: never request portal names on the user's session bus.
        use std::io::BufRead;
        use std::process::{Command, Stdio};
        struct Bus(std::process::Child);
        impl Drop for Bus {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut bus = Bus(Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon required for security tests"));
        let mut address = String::new();
        std::io::BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let owner = zbus::blocking::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .unwrap();
        let other = zbus::blocking::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .unwrap();
        owner
            .request_name("org.freedesktop.portal.Desktop")
            .unwrap();
        for client in [&owner, &other] {
            let message = zbus::Message::method_call("/portal", "Screenshot")
                .unwrap()
                .sender(client.unique_name().unwrap())
                .unwrap()
                .build(&())
                .unwrap();
            assert!(matches!(
                frontend(&other, &message.header()),
                Err(fdo::Error::AccessDenied(_))
            ));
        }
    }
}
