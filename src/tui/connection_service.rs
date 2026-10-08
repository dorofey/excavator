//! Blocking saved-connection operations. Call only from background workers.
//! Secrets stay in this process and the existing OS credential store.
use crate::{
    connections::{self, ConnectionRecord, ConnectionSecrets, Protocol},
    domain::Location,
    providers::{self, ProviderRegistry},
};
use std::collections::BTreeSet;

pub fn load_saved() -> Result<Vec<ConnectionRecord>, String> {
    connections::load()
}

pub fn location_for(record: &ConnectionRecord) -> Location {
    match record.protocol {
        Protocol::Sftp => Location::Sftp {
            connection: record.id.clone(),
            path: record.root.clone(),
        },
        Protocol::Ftps => Location::Ftps {
            connection: record.id.clone(),
            path: record.root.clone(),
        },
        Protocol::S3 => Location::S3 {
            connection: record.id.clone(),
            bucket: record.bucket.clone(),
            key: record.root.clone(),
            prefix: true,
        },
    }
}

pub fn prepare(location: &Location) -> Result<ProviderRegistry, String> {
    prepare_locations(std::slice::from_ref(location))
}

/// Resolve every remote endpoint needed by the source process before execution.
pub fn prepare_locations(locations: &[Location]) -> Result<ProviderRegistry, String> {
    let ids: BTreeSet<&str> = locations
        .iter()
        .filter_map(Location::connection_id)
        .collect();
    if ids.is_empty() {
        return Ok(ProviderRegistry::default());
    }
    let saved = load_saved()?;
    let mut pairs = Vec::new();
    let mut trusted = Vec::new();
    for id in ids {
        let record = saved
            .iter()
            .find(|record| record.id == id)
            .ok_or("Connection was removed; choose another saved connection")?;
        record.validate()?;
        for location in locations
            .iter()
            .filter(|location| location.connection_id() == Some(id))
        {
            let expected = match location {
                Location::Sftp { .. } => Protocol::Sftp,
                Location::Ftps { .. } => Protocol::Ftps,
                Location::S3 { .. } => Protocol::S3,
                Location::Local(_) => unreachable!(),
            };
            if record.protocol != expected {
                return Err("Location protocol does not match saved connection".into());
            }
        }
        let secret = if record.protocol == Protocol::Sftp && !record.ssh_key_path.is_empty() {
            connections::secrets_or_empty_for_key(id)?
        } else {
            connections::secrets(id)?
        };
        if record.protocol == Protocol::Sftp
            && let Some(fingerprint) = connections::known_host(record)?
        {
            trusted.push((id.to_string(), fingerprint));
        }
        pairs.push((record.clone(), secret));
    }
    let mut registry = ProviderRegistry::with_connections(pairs);
    for (id, fingerprint) in trusted {
        registry = registry.with_trusted_host(&id, fingerprint);
    }
    Ok(registry)
}

/// None preserves existing credentials. Some replaces them in Keychain.
pub fn save(record: &ConnectionRecord, secrets: Option<ConnectionSecrets>) -> Result<(), String> {
    connections::save(record, &secrets)
}

pub fn remove(id: &str) -> Result<(), String> {
    connections::remove(id)
}

fn saved_record(id: &str) -> Result<ConnectionRecord, String> {
    load_saved()?
        .into_iter()
        .find(|record| record.id == id)
        .ok_or_else(|| "Connection was removed".into())
}

#[derive(Clone, Debug)]
pub struct HostReview {
    pub record: ConnectionRecord,
    pub fingerprint: String,
    pub previous: Option<String>,
}

/// Probe before credential lookup, so first-connect trust can be reviewed even
/// when Keychain credentials are missing. A changed key is never accepted here.
pub fn probe(id: &str) -> Result<HostReview, String> {
    let record = saved_record(id)?;
    let fingerprint = providers::probe_host(&record)?;
    let previous = connections::known_host(&record)?;
    Ok(HostReview {
        record,
        fingerprint,
        previous,
    })
}

/// Recheck the observed fingerprint at confirmation. The shared store rejects
/// replacement of an existing trusted key until it has explicitly been forgotten.
pub fn trust(id: &str, fingerprint: &str) -> Result<(), String> {
    let record = saved_record(id)?;
    let observed = providers::probe_host(&record)?;
    if observed != fingerprint {
        return Err("SSH fingerprint changed during review; inspect the host again".into());
    }
    connections::trust_host(&record, fingerprint)
}

/// Prefer this confirmation API: bind approval to the reviewed saved endpoint.
pub fn trust_review(review: &HostReview) -> Result<(), String> {
    let current = saved_record(&review.record.id)?;
    if current.protocol != review.record.protocol
        || current.host != review.record.host
        || current.port != review.record.port
    {
        return Err("Connection endpoint changed during review; inspect the host again".into());
    }
    let observed = providers::probe_host(&review.record)?;
    if observed != review.fingerprint {
        return Err("SSH fingerprint changed during review; inspect the host again".into());
    }
    let current = saved_record(&review.record.id)?;
    if current.protocol != review.record.protocol
        || current.host != review.record.host
        || current.port != review.record.port
    {
        return Err("Connection endpoint changed during review; inspect the host again".into());
    }
    connections::trust_host(&review.record, &review.fingerprint)
}

/// Caller must present explicit destructive trust-reset confirmation first.
pub fn forget_host(id: &str) -> Result<(), String> {
    connections::forget_host(&saved_record(id)?)
}
