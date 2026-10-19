use anki::collection::Collection;
use anki::prelude::I18n;
use anki::sync::collection::normal::SyncActionRequired;
use anki::sync::collection::normal::SyncOutput;
use anki::sync::login::SyncAuth;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;

/// Two-way merge sync with AnkiWeb (or whichever server `auth` points at),
/// exactly as desktop's Sync button does when no full sync is needed.
/// rslib runs it in one transaction and rolls back on any error.
pub fn normal_sync(col: &mut Collection, auth: SyncAuth) -> Result<SyncOutput> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime for syncing")?;
    // http1 only, matching desktop's client (nginx doesn't proxy http2).
    let client = reqwest::Client::builder()
        .http1_only()
        .build()
        .context("failed to build the HTTP client for syncing")?;
    // rslib's `Display` for sync errors is just the variant name; its
    // `message` gives the same text desktop shows (e.g. a wrong login).
    runtime
        .block_on(col.normal_sync(auth, client))
        .map_err(|err| anyhow::anyhow!("sync failed: {}", err.message(&I18n::template_only())))
}

/// Describe a finished sync for the user, or error if the server needs a
/// one-way full sync instead: picking a direction can throw away either
/// side's changes, so that's left to desktop's own prompt for now.
pub fn describe_sync(output: &SyncOutput) -> Result<String> {
    match output.required {
        SyncActionRequired::NoChanges => {}
        SyncActionRequired::FullSyncRequired {
            upload_ok,
            download_ok,
        } => {
            let options = match (upload_ok, download_ok) {
                (true, true) => "upload or download",
                (true, false) => "upload",
                (false, true) => "download",
                (false, false) => "full sync",
            };
            bail!(
                "AnkiWeb needs a one-way full sync ({options}), which this CLI doesn't do; \
                 sync from Anki desktop and choose a direction there. Nothing was changed."
            );
        }
        SyncActionRequired::NormalSyncRequired => {
            bail!("sync ended without merging changes; try again")
        }
    }
    let mut out = String::from("synced with AnkiWeb\n");
    if !output.server_message.is_empty() {
        out.push_str(&format!(
            "message from AnkiWeb: {}\n",
            output.server_message
        ));
    }
    if let Some(endpoint) = &output.new_endpoint {
        // Desktop would save this to prefs21.db; we only read that file, but
        // desktop gets the same redirect on its own next sync.
        out.push_str(&format!(
            "note: AnkiWeb redirected this account to {endpoint}\n"
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anki::collection::CollectionBuilder;
    use anki::sync::http_server::SimpleServer;
    use anki::sync::http_server::SyncServerConfig;
    use anki::sync::http_server::default_ip_header;

    use super::*;
    use crate::cards;
    use crate::prefs::SyncPrefs;
    use crate::prefs::sync_auth;

    /// rslib's sync server derives this host key from `user:pass`.
    const HKEY: &str = "b2619aa1529dfdc4248e6edbf3c1b2a2b014cf6d";

    /// Start rslib's own sync server on a background thread, returning its
    /// endpoint URL.
    fn start_server(base_folder: &Path) -> String {
        // SAFETY: the only test that touches the environment, and rslib's
        // server reads its user list from it.
        unsafe { std::env::set_var("SYNC_USER1", "user:pass") };
        let config = SyncServerConfig {
            host: "127.0.0.1".parse().unwrap(),
            port: 0,
            base_folder: base_folder.into(),
            ip_header: default_ip_header(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let (addr, server) = SimpleServer::make_server(config).await.unwrap();
                tx.send(addr).unwrap();
                server.await.unwrap();
            });
        });
        format!("http://{}/", rx.recv().unwrap())
    }

    fn auth(endpoint: &str) -> SyncAuth {
        sync_auth(&SyncPrefs {
            sync_key: Some(HKEY.into()),
            current_sync_url: Some(endpoint.into()),
            ..Default::default()
        })
        .unwrap()
    }

    fn add_basic_note(col: &mut Collection, front: &str) {
        let fields = vec![
            ("Front".to_string(), front.to_string()),
            ("Back".to_string(), "back".to_string()),
        ];
        cards::add(col, "Basic", "Default", &fields, &[]).unwrap();
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn normal_sync_pushes_local_changes_to_the_server() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = start_server(&dir.path().join("server"));
        let local_path = dir.path().join("local.anki2");

        // A first sync against an empty server can only be a full one, which
        // we refuse to pick a direction for.
        let mut col = CollectionBuilder::new(&local_path).build().unwrap();
        add_basic_note(&mut col, "one");
        let output = normal_sync(&mut col, auth(&endpoint)).unwrap();
        assert!(describe_sync(&output).is_err());

        // Seed the server the way desktop would, then make a change that a
        // normal sync has to carry up.
        block_on(col.full_upload(auth(&endpoint), reqwest::Client::new())).unwrap();
        let mut col = CollectionBuilder::new(&local_path).build().unwrap();
        add_basic_note(&mut col, "two");
        let output = normal_sync(&mut col, auth(&endpoint)).unwrap();
        assert!(describe_sync(&output).is_ok());

        // A fresh download of the server's copy now has both notes.
        let other_path = dir.path().join("other.anki2");
        let other = CollectionBuilder::new(&other_path).build().unwrap();
        block_on(other.full_download(auth(&endpoint), reqwest::Client::new())).unwrap();
        let mut other = CollectionBuilder::new(&other_path).build().unwrap();
        assert_eq!(other.search_notes_unordered("").unwrap().len(), 2);
    }
}
