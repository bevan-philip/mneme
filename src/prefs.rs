use std::ffi::OsStr;
use std::path::Path;

use anki::sync::login::SyncAuth;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use rusqlite::Connection;
use rusqlite::OpenFlags;
use rusqlite::OptionalExtension;
use serde::Deserialize;

/// What we need from a profile's entry in Anki desktop's `prefs21.db` to
/// sync as that profile. Keys and defaults mirror `qt/aqt/profiles.py`;
/// every other key in the pickled dict is ignored.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct SyncPrefs {
    #[serde(rename = "syncKey")]
    pub sync_key: Option<String>,
    #[serde(rename = "syncUser")]
    pub sync_user: Option<String>,
    /// The last endpoint AnkiWeb redirected desktop to.
    #[serde(rename = "currentSyncUrl")]
    pub current_sync_url: Option<String>,
    /// A self-hosted sync server set in desktop's preferences.
    #[serde(rename = "customSyncUrl")]
    pub custom_sync_url: Option<String>,
    #[serde(rename = "networkTimeout")]
    pub network_timeout: Option<u32>,
}

/// Desktop's `network_timeout()` fallback.
const DEFAULT_NETWORK_TIMEOUT_SECS: u32 = 60;

/// Decode one profile's pickled prefs dict. The dict can hold Qt objects
/// pickled via `sip` (window geometry and the like) that only resolve inside
/// PyQt; those are replaced with `None` rather than failing, since none of
/// them are fields we read.
pub fn parse_sync_prefs(data: &[u8]) -> Result<SyncPrefs> {
    serde_pickle::from_slice(
        data,
        serde_pickle::DeOptions::new().replace_unresolved_globals(),
    )
    .context("failed to decode the profile's pickled preferences")
}

/// Build rslib's sync credentials the same way desktop's `sync_auth()` does:
/// the redirected endpoint wins over a custom server, and no endpoint at all
/// means AnkiWeb's default.
pub fn sync_auth(prefs: &SyncPrefs) -> Result<SyncAuth> {
    let Some(hkey) = prefs.sync_key.clone().filter(|key| !key.is_empty()) else {
        bail!(
            "this profile isn't logged in to AnkiWeb; log in once from Anki desktop's Sync button"
        );
    };
    let endpoint = prefs
        .current_sync_url
        .clone()
        .or_else(|| prefs.custom_sync_url.clone())
        .filter(|url| !url.is_empty());
    let auth = anki_proto::sync::SyncAuth {
        hkey,
        endpoint,
        io_timeout_secs: Some(
            prefs
                .network_timeout
                .filter(|secs| *secs > 0)
                .unwrap_or(DEFAULT_NETWORK_TIMEOUT_SECS),
        ),
    };
    Ok(auth.try_into()?)
}

/// Read a profile's sync preferences from `<anki2_root>/prefs21.db`,
/// read-only: we never write back, since re-pickling would drop the Qt
/// values `parse_sync_prefs` can't decode.
pub fn read_sync_prefs(anki2_root: &Path, profile: &OsStr) -> Result<SyncPrefs> {
    let path = anki2_root.join("prefs21.db");
    let Some(profile) = profile.to_str() else {
        bail!(
            "profile name \"{}\" isn't valid UTF-8, so it can't be looked up in {}",
            profile.to_string_lossy(),
            path.display()
        );
    };
    let db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open {}", path.display()))?;
    let data: Option<Vec<u8>> = db
        .query_row(
            "select cast(data as blob) from profiles where name = ?",
            [profile],
            |row| row.get(0),
        )
        .optional()
        .with_context(|| format!("failed to read {}", path.display()))?;
    let Some(data) = data else {
        bail!("no profile named \"{profile}\" in {}", path.display());
    };
    parse_sync_prefs(&data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A profile dict as desktop pickles it (protocol 4), including a
    /// `QByteArray` pickled through `PyQt6.sip`, generated with:
    /// `pickle.dumps({"mainWindowGeom": <QByteArray>, "syncKey": "0123abcd",
    /// "syncUser": "me@example.com", "hostNum": 3, "currentSyncUrl": None,
    /// "customSyncUrl": None, "networkTimeout": 90, "numBackups": 30,
    /// "lastOptimize": 1700000000}, protocol=4)`.
    const DESKTOP_PROFILE: &[u8] = b"\
    \x80\x04\x95\xf2\x00\x00\x00\x00\x00\x00\x00\x7d\x94\x28\x8c\x0e\x6d\x61\x69\x6e\x57\x69\x6e\x64\
    \x6f\x77\x47\x65\x6f\x6d\x94\x8c\x09\x50\x79\x51\x74\x36\x2e\x73\x69\x70\x94\x8c\x0e\x5f\x75\x6e\
    \x70\x69\x63\x6b\x6c\x65\x5f\x74\x79\x70\x65\x94\x93\x94\x8c\x0c\x50\x79\x51\x74\x36\x2e\x51\x74\
    \x43\x6f\x72\x65\x94\x8c\x0a\x51\x42\x79\x74\x65\x41\x72\x72\x61\x79\x94\x43\x02\x01\x02\x94\x85\
    \x94\x87\x94\x52\x94\x8c\x07\x73\x79\x6e\x63\x4b\x65\x79\x94\x8c\x08\x30\x31\x32\x33\x61\x62\x63\
    \x64\x94\x8c\x08\x73\x79\x6e\x63\x55\x73\x65\x72\x94\x8c\x0e\x6d\x65\x40\x65\x78\x61\x6d\x70\x6c\
    \x65\x2e\x63\x6f\x6d\x94\x8c\x07\x68\x6f\x73\x74\x4e\x75\x6d\x94\x4b\x03\x8c\x0e\x63\x75\x72\x72\
    \x65\x6e\x74\x53\x79\x6e\x63\x55\x72\x6c\x94\x4e\x8c\x0d\x63\x75\x73\x74\x6f\x6d\x53\x79\x6e\x63\
    \x55\x72\x6c\x94\x4e\x8c\x0e\x6e\x65\x74\x77\x6f\x72\x6b\x54\x69\x6d\x65\x6f\x75\x74\x94\x4b\x5a\
    \x8c\x0a\x6e\x75\x6d\x42\x61\x63\x6b\x75\x70\x73\x94\x4b\x1e\x8c\x0c\x6c\x61\x73\x74\x4f\x70\x74\
    \x69\x6d\x69\x7a\x65\x94\x4a\x00\xf1\x53\x65\x75\x2e";

    #[test]
    fn decodes_a_desktop_profile_despite_qt_values() {
        assert_eq!(
            parse_sync_prefs(DESKTOP_PROFILE).unwrap(),
            SyncPrefs {
                sync_key: Some("0123abcd".into()),
                sync_user: Some("me@example.com".into()),
                current_sync_url: None,
                custom_sync_url: None,
                network_timeout: Some(90),
            }
        );
    }

    #[test]
    fn auth_prefers_the_redirected_endpoint_over_a_custom_one() {
        let prefs = SyncPrefs {
            sync_key: Some("key".into()),
            current_sync_url: Some("https://sync2.example.net/".into()),
            custom_sync_url: Some("https://custom.example.net/".into()),
            ..Default::default()
        };
        let auth = sync_auth(&prefs).unwrap();
        assert_eq!(auth.hkey, "key");
        assert_eq!(
            auth.endpoint.unwrap().as_str(),
            "https://sync2.example.net/"
        );
        assert_eq!(auth.io_timeout_secs, Some(DEFAULT_NETWORK_TIMEOUT_SECS));
    }

    #[test]
    fn auth_defaults_to_ankiweb_and_requires_a_login() {
        let prefs = SyncPrefs {
            sync_key: Some("key".into()),
            ..Default::default()
        };
        assert!(sync_auth(&prefs).unwrap().endpoint.is_none());
        assert!(sync_auth(&SyncPrefs::default()).is_err());
    }

    #[test]
    fn reads_a_profile_row_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let db = Connection::open(dir.path().join("prefs21.db")).unwrap();
        db.execute_batch(
            "create table profiles (name text primary key collate nocase, data blob not null)",
        )
        .unwrap();
        db.execute(
            "insert into profiles values ('User 1', ?)",
            [DESKTOP_PROFILE],
        )
        .unwrap();
        drop(db);

        let prefs = read_sync_prefs(dir.path(), OsStr::new("user 1")).unwrap();
        assert_eq!(prefs.sync_key.as_deref(), Some("0123abcd"));
        assert!(read_sync_prefs(dir.path(), OsStr::new("Other")).is_err());
    }
}
