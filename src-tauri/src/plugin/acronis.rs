//! Real plugin implementation for Acronis Cyber Protect Cloud (backup /
//! cyber-protection platform, developer.acronis.com), see
//! `docs/PLUGIN_ARCHITECTURE.md` section "Acronis-Plugin". Ninth real
//! integration after NinjaOne, Level.io, Snipe-IT, Microsoft Intune, Iru
//! (Kandji), Jamf Pro, Apple Business Manager, and Tactical RMM, and
//! DIFFERENT IN KIND from all eight of those: every prior integration
//! surfaces device/asset inventory (an RMM's or MDM's list of managed
//! machines). Acronis Cyber Protect Cloud is not an RMM/MDM; it is a
//! backup platform, and this plugin's whole job is to surface BACKUP
//! HEALTH per customer/device (is backup currently OK, warning, or
//! failing right now), never device inventory management. Concretely,
//! `AcronisResource.backup_status` (an Alert Manager `severity` value, see
//! below) is the entire payload this plugin adds beyond bare identity;
//! there is no "install agent"/"run script"/inventory-management concept
//! here at all.
//!
//! Structurally, though, it plugs into the exact same `Plugin` trait and
//! the exact same connection/mapping/sync/link pattern as every other
//! plugin here: like Tactical RMM/NinjaOne/Snipe-IT/Jamf, a connection
//! (one Acronis "API client") can see MULTIPLE tenants ("customers" in
//! Acronis's own tenant hierarchy), so the same connection+mapping-table
//! pattern applies (`AcronisConnectionMeta`/`AcronisTenantMapping`,
//! mapping granularity at the tenant-of-kind-"customer" level, see below).
//! Authentication is OAuth2 client-credentials, like NinjaOne/Intune, plus
//! one extra discovery step unique to this plugin (see below).
//!
//! Every fact below is taken verbatim from the verified research this
//! plugin was built from (developer.acronis.com's own docs plus three
//! live-fetched OpenAPI specs), not guessed:
//!
//! - **Authentication**: OAuth2 client-credentials grant, `POST
//!   {datacenter_url}/api/2/idp/token`, header `Authorization: Basic
//!   base64(client_id:client_secret)`, form body
//!   `grant_type=client_credentials`. Response `{"access_token", "token_type":
//!   "bearer", "expires_on", ...}`, used as `Authorization: Bearer
//!   <access_token>` on every subsequent call. Like every other plugin here,
//!   the token is fetched fresh per real operation, never cached/refreshed
//!   across calls (this app calls plugin methods rarely/manually, not in a
//!   hot loop, the same reasoning as `plugin::ninja`/`plugin::intune`).
//! - **`datacenter_url` is entirely user-supplied**: registering an API
//!   client in Acronis's management console (done once, out-of-band, by
//!   the user, before configuring this plugin) hands back THREE values at
//!   once: `client_id`, `client_secret`, AND a `datacenter_url` (e.g.
//!   `https://eu2-cloud.acronis.com`). No fixed/enumerable list of
//!   datacenters exists in the docs, so, like Tactical RMM's `base_url`,
//!   the user must type it in. This makes it a genuine THREE-value
//!   credential, unlike every other plugin here so far, but
//!   `datacenter_url` is NOT itself a secret (it is shown openly in
//!   Acronis's own UI, just like a base URL), so per the credential
//!   principle (`docs/PLUGIN_ARCHITECTURE.md`, "Credential-Prinzip") it
//!   lives in `AcronisConnectionMeta.datacenter_url` (`config.toml`), NOT
//!   in the keyring secret, exactly the same non-secret-metadata-in-config
//!   principle already used for `base_url` on Tactical RMM/NinjaOne/
//!   Snipe-IT/Iru/Jamf. `AcronisCredentials` therefore stays a clean
//!   two-value JSON (`client_id`, `client_secret`), just like
//!   `plugin::ninja::NinjaCredentials`.
//! - **Base URL, a real gotcha, corrected after a live 404**: Acronis
//!   splits its platform into separate APIs with DIFFERENT base paths
//!   under the same `datacenter_url`, verified against
//!   developer.acronis.com after an initial (wrong) assumption that every
//!   endpoint shared one prefix caused real 404s against a live tenant.
//!   Account Management v2 (`idp/token`, `clients/{client_id}`,
//!   `tenants`) lives under `{datacenter_url}/api/2` (`{base}` below).
//!   Resource and Policy Management v4 and Alert Manager v1 do NOT --
//!   they live directly under `{datacenter_url}/api` (no `2/` segment),
//!   a separate `{resource_base}` used only by `fetch_all_resources`/
//!   `fetch_all_severities`/`get_resource_statuses`.
//! - **Extra discovery step, unique to this plugin**: after obtaining a
//!   bearer token, `GET {datacenter_url}/api/2/clients/{client_id}`
//!   (Bearer auth) returns `{"tenant_id": "<uuid>", "type": "api_client",
//!   ...}`, the API client's own ROOT tenant, the anchor `/tenants` needs
//!   to walk the full accessible tenant tree in one call (see below). Done
//!   once per real operation (`test_credentials`, `list_tenants`, one per
//!   mapped tenant inside a `sync_acronis_connection` run), like the token
//!   itself, not cached across calls.
//! - **Tenants (the mapping entity)**: `GET {base}/tenants`, query params
//!   `subtree_root_id=<the discovered root tenant id>`, `lod=full`, cursor
//!   `after`. Response envelope (NOT a bare array, unlike Tactical RMM's
//!   `/clients/`): `{"timestamp", "paging": {"cursors": {"after": "..."}},
//!   "items": [{"id", "parent_id", "name", "kind"}]}`. `kind` is one of
//!   `root | partner | folder | customer | unit`; ONLY `kind ==
//!   "customer"` tenants are valid mapping targets (organizational
//!   containers otherwise, not real customer accounts), mirroring how
//!   Tactical RMM's Client level (not Site) is the mapping granularity,
//!   just gated on a `kind` field instead of a hierarchy level. `map_tenant`
//!   (single item, unfiltered) and `filter_customer_tenants` (the `kind ==
//!   "customer"` filter) are deliberately separate, independently
//!   pure/testable functions; see their own doc comments.
//! - **Backup status per device, a genuine scope decision, not a
//!   guess**: the docs don't offer one single obvious "is backup OK"
//!   field, and two separate APIs are involved:
//!   1. **Resource identity**: `GET {resource_base}/resource_management/v4/resources`,
//!      query param `tenant_id=<comma-joined subtree of a mapped tenant's
//!      id + all its descendant tenants/units>` (see
//!      `fetch_tenant_subtree_csv`/`collect_subtree_tenant_ids` -- a real,
//!      live-verified fix: this filter is EXACT-MATCH per tenant, NOT
//!      recursive, confirmed against developer.acronis.com's real OpenAPI
//!      spec, after an initial assumption that a single mapped tenant's
//!      own id was enough caused a live account's resources to be
//!      massively under-counted, since most of them were actually
//!      registered under child units/sites beneath the mapped customer
//!      tenant, not the customer tenant node itself). ALSO sends
//!      `is_group=false` and `type=resource.machine` -- another real,
//!      live-verified fix: without these, the same endpoint also returns
//!      dynamic/static resource GROUPS ("All", "All virtual machines",
//!      "All PostgreSQL databases" -- confirmed against
//!      developer.acronis.com's `is_group` query parameter, which is
//!      request-only, never a response field) and non-machine resource
//!      types (`resource.mssql_server` database instances, shown to a
//!      real user as `mssql://...` entries) mixed in with real devices.
//!      Cursor `before`/`after`. Response `{"items": [{"id", "name",
//!      "agent_id", "external_id", "type", "aggregate_id"}], "paging":
//!      {"cursors": {...}}}`. This plugin uses `id` as
//!      `AcronisResource::external_id` and `name` as the display name,
//!      NOT the resource's own, confusingly-named `external_id` field (a
//!      different, Acronis-internal concept, not this app's identity join
//!      key) and NOT `agent_id` (Acronis's own agent concept, irrelevant
//!      here). `dedupe_resources_by_aggregate_id` runs right after this
//!      fetch: the same physical/virtual machine can be reported as
//!      multiple separate resource entries (different agents/backup
//!      roles protecting it -- see `aggregate_id`/`aggregation_status`/
//!      `aspects` in Acronis's own Resource schema), confirmed against a
//!      real account that showed e.g. "SRV-LF-SU-20" and
//!      "SRV-LF-SU-20.lflohmar.local" as two separate rows for the same
//!      machine.
//!   2. **Backup health**: `GET {resource_base}/alert_manager/v1/resource_status`,
//!      filtered by the EXACT resource IDs found in step 1
//!      (`id=<single id>` or `id=or(id1,id2,...)` for multiple, Acronis's
//!      own filter-expression syntax) -- a corrected, real mistake in an
//!      earlier version of this module, which sent an undocumented,
//!      silently-ignored `tenant=<id>` parameter; verified against
//!      developer.acronis.com's real OpenAPI spec that this endpoint has
//!      NO `tenant`/`tenant_id` parameter at all, only `id` and
//!      `embed_alert`. Response: `{"items": [{"id":
//!      "<resourceId>", "severity": "ok|information|warning|error|critical",
//!      "alert": {...}}]}`. THIS is the "is backup currently fine"
//!      answer for this plugin. `join_resources_with_severity` joins on
//!      `id` (Acronis's resource ID, the same value as #1's `id`) against
//!      `AcronisResource::external_id`, copying `severity` into
//!      `AcronisResource::backup_status` verbatim (free-form string
//!      passthrough, like Tactical RMM's `status`/`platform`, no Rust
//!      enum, so a future new severity value doesn't break parsing). A
//!      resource with NO matching alert-manager entry (never backed up,
//!      or not protected at all) gets `backup_status: None`, a legitimate,
//!      expected case, not an error. If step 1 finds zero resources, step
//!      2 is skipped entirely (no `id` filter can be built from an empty
//!      set).
//!   3. **Deliberately NOT attempted**: filtering
//!      `resource_management/v4/resource_statuses`'s `policies[]` array by
//!      a specific backup-policy-type CTI string. That string was never
//!      confirmed against real docs/specs, and guessing it wrong would
//!      silently show meaningless data to the user (worse than showing
//!      nothing). For a richer per-resource payload, this plugin instead
//!      calls `GET {resource_base}/resource_management/v4/resource_statuses?tenant_id=<id>`
//!      UNFILTERED and passes the raw JSON straight through (see
//!      `get_resource_statuses` below), the same "raw, free-form JSON,
//!      caller/UI interprets it" contract every other plugin here uses for
//!      `Plugin::get_system_details`.
//! - **A genuine mismatch this plugin had to design around**: Acronis has
//!   NO verified single-resource detail endpoint. `resource_statuses` is
//!   always TENANT-scoped, never resource-scoped, but the `Plugin`
//!   trait's `get_system_details(&self, credentials, external_id)` method
//!   has no room for a tenant parameter. Rather than guess at an
//!   unconfirmed single-resource endpoint, `Plugin::get_system_details` for
//!   `AcronisPlugin` deliberately returns `PluginError::UnexpectedResponse`
//!   explaining exactly this, an honest gap, analogous to Tactical RMM's
//!   missing web-dashboard link or Snipe-IT's always-`None` `hostname`/
//!   `ip_address` (see their own module docs), not a guess dressed up as a
//!   feature. The real, richer payload is available via the inherent
//!   method `get_resource_statuses(&self, credentials, tenant_id)`
//!   instead, which DOES take a tenant ID: `commands::acronis` calls
//!   this directly (never the trait method) for
//!   `sync_acronis_connection`/`link_system_to_acronis`/
//!   `get_acronis_system_details`, all of which already carry tenant
//!   context from the tenant mapping they operate on. This is the one
//!   genuine, deliberate deviation from every other plugin here (where the
//!   trait method IS the real implementation), flagged here explicitly,
//!   see also `docs/PLUGIN_ARCHITECTURE.md`. `Plugin::list_systems` has the
//!   same underlying problem (no verified "all tenants at once" resources
//!   endpoint, and no tenant parameter on the trait method) and is
//!   therefore also deliberately narrow: it returns an empty list.
//!   `commands::acronis` never calls it either, using the tenant-scoped
//!   inherent method `list_resources_with_status` instead, once per mapped
//!   tenant.
//! - **Pagination**: cursor-based, `paging.cursors.after` in the response,
//!   passed back as the query param `after` for the next page; an
//!   absent/empty `after` means no more pages. Same shape for
//!   `/tenants`, `/resource_management/v4/resources`, and
//!   `/alert_manager/v1/resource_status` (consulted per-response via
//!   `parse_paged_items`, not assumed as one hardcoded global constant, in
//!   case a future Acronis release changes one endpoint's envelope without
//!   the others).
//! - **Rate limits**: not documented anywhere reachable. Noted here, no
//!   special handling (no backoff/retry), same honest gap as every other
//!   plugin's rate-limit situation in this codebase.
//! - **No web dashboard deep-link**: not documented, so it is left out,
//!   same honest omission as `plugin::tacticalrmm`.
//! - **HTTP client**: `ureq` 3.4.1, synchronous, the same dependency as
//!   every other plugin here.
//! - **Credential encoding**: two secret values (`client_id`,
//!   `client_secret`); `datacenter_url` deliberately excluded (see
//!   above), JSON-encoded exactly like `plugin::ninja::NinjaCredentials`.

use base64::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Protection against a misbehaving remote end (an `after` cursor that
/// never stops appearing): more than `MAX_PAGES` pages are not fetched per
/// paginated call, the same defensive convention as
/// `plugin::intune::MAX_PAGES`/`plugin::level::MAX_PAGES`.
const MAX_PAGES: usize = 50;

/// Non-secret metadata of an Acronis connection, as stored in
/// `config.toml` (`Config::acronis_connections`). The OAuth2 client
/// ID/secret belong, per the credential principle, exclusively in the OS
/// keyring, never here, but `datacenter_url` deliberately lives here,
/// NOT in the keyring secret (see module docs: it is not itself sensitive,
/// exactly like Tactical RMM's `base_url`). Deliberately WITHOUT
/// `customer_id`: a connection is one Acronis API client, not a local
/// customer; which tenant within it corresponds to which local customer is
/// tracked granularly in `AcronisTenantMapping`/
/// `Config::acronis_tenant_mappings`, exactly the same principle as
/// `plugin::tacticalrmm::TacticalRmmConnectionMeta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcronisConnectionMeta {
    pub id: String,
    pub label: String,
    pub datacenter_url: String,
}

/// Mapping of a single Acronis tenant (`kind == "customer"`, within a
/// connection) to a local customer. Lives in
/// `Config::acronis_tenant_mappings`, not in `AcronisConnectionMeta`: a
/// connection can see multiple customer tenants, each mapped independently
/// (or left unmapped), exactly analogous to
/// `plugin::tacticalrmm::TacticalRmmClientMapping`. `tenant_id` is
/// Acronis's own tenant UUID (from `GET /tenants`); `tenant_name` is
/// stored alongside it so a UI list can display a readable name without
/// another live call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcronisTenantMapping {
    pub connection_id: String,
    pub tenant_id: String,
    pub tenant_name: String,
    pub customer_id: i64,
}

/// A single tenant from `GET /tenants`, kept as its own small type
/// (separate from `ExternalSystem`, which is for resources/devices).
/// `kind` is deliberately kept as a field (not filtered away at this
/// level) so the command layer/frontend can apply the `kind == "customer"`
/// mapping-candidate filter explicitly (see `filter_customer_tenants`),
/// analogous to `plugin::tacticalrmm::TacticalRmmClient`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcronisTenant {
    pub id: String,
    pub name: String,
    pub kind: String,
}

/// A single backup-monitored resource (device), the result of joining
/// `GET /resource_management/v4/resources` (identity) with
/// `GET /alert_manager/v1/resource_status` (backup health); see module
/// docs on why both calls are needed. `backup_status` is Acronis's own
/// Alert Manager `severity` string
/// (`"ok"`/`"information"`/`"warning"`/`"error"`/`"critical"`), passed
/// through verbatim. `None` if no alert-manager entry exists for this
/// resource at all (e.g. never backed up / not protected), which is a
/// legitimate case, not an error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcronisResource {
    pub external_id: String,
    pub name: String,
    pub tenant_id: String,
    pub tenant_name: String,
    pub backup_status: Option<String>,
}

/// The two secret values an Acronis connection needs to authenticate.
/// `PluginCredentials.secret` is, per the trait contract, a single opaque
/// string that the plugin interprets itself, here encoded as JSON
/// (`serde_json::to_string`/`from_str`), analogous to
/// `plugin::ninja::NinjaCredentials`. Deliberately WITHOUT
/// `datacenter_url`; see module docs and `AcronisConnectionMeta`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcronisCredentials {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct ClientInfoResponse {
    tenant_id: String,
}

/// A plugin object for exactly one configured Acronis connection. `id`
/// here is already the fully qualified identifier
/// (`"acronis:<connection_id>"`), so that `Plugin::id()` works unmodified
/// as the `plugin_id`/keyring account (see trait documentation in
/// `plugin::mod`).
pub struct AcronisPlugin {
    id: String,
    datacenter_url: String,
}

impl AcronisPlugin {
    pub fn new(id: String, datacenter_url: String) -> Self {
        Self { id, datacenter_url }
    }

    /// Live fetch of this connection's tenants, narrowed to `kind ==
    /// "customer"` (see module docs): the mapping UI's candidate list.
    /// Resolves the root tenant id (the `/clients/{client_id}` discovery
    /// call) first, then walks every page of `/tenants` under it.
    pub fn list_tenants(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<AcronisTenant>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &self.datacenter_url, &creds)?;
        let root_tenant_id =
            fetch_root_tenant_id(&agent, &self.datacenter_url, &token, &creds.client_id)?;
        let raw_items = fetch_all_tenants(&agent, &self.datacenter_url, &token, &root_tenant_id)?;
        let tenants: Vec<AcronisTenant> = raw_items.iter().filter_map(map_tenant).collect();
        Ok(filter_customer_tenants(&tenants))
    }

    /// Live fetch of a single mapped tenant's resources, joined with their
    /// current backup severity (see module docs on the two-API join).
    /// Richer than the trait method `list_systems`, which is deliberately
    /// empty here (see module docs). Resolves `tenant_id`'s full subtree
    /// first (see `collect_subtree_tenant_ids`) since resources can be
    /// registered under a child unit/site of the mapped customer tenant,
    /// not only the customer tenant itself -- a real, live-verified fix,
    /// not a defensive guess.
    pub fn list_resources_with_status(
        &self,
        credentials: &PluginCredentials,
        tenant_id: &str,
        tenant_name: &str,
    ) -> Result<Vec<AcronisResource>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &self.datacenter_url, &creds)?;

        let subtree_csv =
            fetch_tenant_subtree_csv(&agent, &self.datacenter_url, &token, &creds, tenant_id)?;
        let raw_resources =
            fetch_all_resources(&agent, &self.datacenter_url, &token, &subtree_csv)?;
        let raw_resources = dedupe_resources_by_aggregate_id(raw_resources);
        let resources: Vec<AcronisResource> = raw_resources
            .iter()
            .filter_map(|value| map_resource(value, tenant_id, tenant_name))
            .collect();

        if resources.is_empty() {
            return Ok(resources);
        }
        let resource_ids: Vec<&str> = resources.iter().map(|r| r.external_id.as_str()).collect();
        let raw_severities =
            fetch_all_severities(&agent, &self.datacenter_url, &token, &resource_ids)?;
        let severities = map_severities_response(&raw_severities);

        Ok(join_resources_with_severity(resources, &severities))
    }

    /// Live fetch of the UNFILTERED, tenant-(sub)tree-wide
    /// `resource_management/v4/resource_statuses` payload; see module
    /// docs on why this is tenant-scoped, not resource-scoped, and why
    /// `Plugin::get_system_details` can't be the real implementation for
    /// this plugin. Raw JSON, passed straight through, the same "caller/UI
    /// interprets it" contract every other plugin's `get_system_details`
    /// uses. Same subtree resolution as `list_resources_with_status`, for
    /// the same reason.
    pub fn get_resource_statuses(
        &self,
        credentials: &PluginCredentials,
        tenant_id: &str,
    ) -> Result<serde_json::Value, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &self.datacenter_url, &creds)?;
        let subtree_csv =
            fetch_tenant_subtree_csv(&agent, &self.datacenter_url, &token, &creds, tenant_id)?;
        let url = format!(
            "{}/api/resource_management/v4/resource_statuses",
            self.datacenter_url.trim_end_matches('/')
        );
        fetch_json(&agent, &url, &token, &[("tenant_id", &subtree_csv)])
    }
}

/// Shared by `list_resources_with_status`/`get_resource_statuses`:
/// resolves the API client's own root tenant, walks the full accessible
/// `/tenants` tree under it (same call `list_tenants` makes, but kept
/// UNFILTERED by `kind` here -- child units/folders are exactly what this
/// needs, not just `kind == "customer"` tenants), then narrows to
/// `mapped_tenant_id`'s own subtree (see `collect_subtree_tenant_ids`) and
/// comma-joins it into the single string value
/// `resource_management/v4`'s `tenant_id` filter expects (see module
/// docs -- confirmed `type: string`, not an array parameter).
fn fetch_tenant_subtree_csv(
    agent: &Agent,
    datacenter_url: &str,
    token: &str,
    creds: &AcronisCredentials,
    mapped_tenant_id: &str,
) -> Result<String, PluginError> {
    let root_tenant_id = fetch_root_tenant_id(agent, datacenter_url, token, &creds.client_id)?;
    let raw_tenants = fetch_all_tenants(agent, datacenter_url, token, &root_tenant_id)?;
    let subtree_ids = collect_subtree_tenant_ids(&raw_tenants, mapped_tenant_id);
    Ok(subtree_ids.join(","))
}

/// Checks a datacenter URL/client ID/client secret triple against Acronis
/// (OAuth2 client-credentials grant, plus the `/clients/{client_id}`
/// discovery call; this alone is a meaningful connectivity+credential
/// test, no extra call needed), without persisting anything. For
/// `commands::acronis::test_acronis_connection`, so users notice a typo
/// before actually creating a connection (writing credentials to the
/// keyring).
pub fn test_credentials(
    datacenter_url: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<(), PluginError> {
    let creds = AcronisCredentials {
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
    };
    let agent = build_agent();
    let token = fetch_access_token(&agent, datacenter_url, &creds)?;
    fetch_root_tenant_id(&agent, datacenter_url, &token, client_id)?;
    Ok(())
}

impl Plugin for AcronisPlugin {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn list_systems(
        &self,
        _credentials: &PluginCredentials,
    ) -> Result<Vec<ExternalSystem>, PluginError> {
        // Deliberately empty, see module docs: Acronis's resources
        // endpoint is documented as always tenant-scoped (no verified
        // "all tenants at once" variant exists), and this trait method has
        // no tenant parameter to scope by. `commands::acronis` never calls
        // this; it always uses the tenant-scoped
        // `list_resources_with_status` inherent method instead, once per
        // mapped tenant (see `commands::acronis::sync_acronis_connection`).
        Ok(Vec::new())
    }

    fn get_system_details(
        &self,
        _credentials: &PluginCredentials,
        _external_id: &str,
    ) -> Result<serde_json::Value, PluginError> {
        // See module docs: Acronis has no verified single-resource detail
        // endpoint, only a tenant-scoped one, and this trait method has
        // no tenant parameter. An honest gap, not a guess.
        // `commands::acronis` never calls this trait method; it calls the
        // inherent `get_resource_statuses(credentials, tenant_id)` method
        // directly, using tenant context it already has from the tenant
        // mapping being operated on.
        Err(PluginError::UnexpectedResponse(
            "Acronis-Ressourcendetails sind nur mandantenbezogen abrufbar (siehe AcronisPlugin::get_resource_statuses), nicht über eine einzelne externe ID".to_string(),
        ))
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // Acronis's API doesn't need to know anything about a local link;
        // it is purely a local bookkeeping concept, see trait documentation.
        // Persisted by the caller via `db::external_refs::upsert`.
        println!("AcronisPlugin({}): verknüpfe lokales System {local_system_id} mit externer ID {external_id}", self.id);
        Ok(())
    }
}

fn parse_credentials(credentials: &PluginCredentials) -> Result<AcronisCredentials, PluginError> {
    serde_json::from_str(&credentials.secret)
        .map_err(|e| PluginError::Authentication(format!("Acronis-Zugangsdaten ungültig: {e}")))
}

fn build_agent() -> Agent {
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build();
    Agent::new_with_config(config)
}

/// `POST {datacenter_url}/api/2/idp/token`, Basic auth over
/// `client_id:client_secret`, form body `grant_type=client_credentials`
/// (see module docs). `send_form` sets its own
/// `application/x-www-form-urlencoded` content type, so it isn't set
/// explicitly here.
fn fetch_access_token(
    agent: &Agent,
    datacenter_url: &str,
    creds: &AcronisCredentials,
) -> Result<String, PluginError> {
    let url = format!("{}/api/2/idp/token", datacenter_url.trim_end_matches('/'));
    let basic_auth = BASE64_STANDARD.encode(format!("{}:{}", creds.client_id, creds.client_secret));
    let mut response = agent
        .post(&url)
        .header("Authorization", format!("Basic {basic_auth}"))
        .send_form([("grant_type", "client_credentials")])
        .map_err(map_ureq_error)?;
    let token: TokenResponse = response.body_mut().read_json().map_err(map_ureq_error)?;
    Ok(token.access_token)
}

/// `GET {datacenter_url}/api/2/clients/{client_id}` (see module docs),
/// the extra discovery step unique to this plugin, resolving the API
/// client's own root tenant id.
fn fetch_root_tenant_id(
    agent: &Agent,
    datacenter_url: &str,
    token: &str,
    client_id: &str,
) -> Result<String, PluginError> {
    let url = format!(
        "{}/api/2/clients/{client_id}",
        datacenter_url.trim_end_matches('/')
    );
    let mut response = agent
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .call()
        .map_err(map_ureq_error)?;
    let info: ClientInfoResponse = response.body_mut().read_json().map_err(map_ureq_error)?;
    Ok(info.tenant_id)
}

/// A single, non-paginated authenticated `GET {url}`, used for
/// `get_resource_statuses`, which passes its response straight through as
/// raw JSON (no merging across pages, unlike `fetch_all_pages` below).
fn fetch_json(
    agent: &Agent,
    url: &str,
    token: &str,
    query: &[(&str, &str)],
) -> Result<serde_json::Value, PluginError> {
    let mut request = agent
        .get(url)
        .header("Authorization", format!("Bearer {token}"));
    for (key, value) in query {
        request = request.query(*key, *value);
    }
    let mut response = request.call().map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

/// Shared cursor-pagination loop for the three list endpoints
/// (`/tenants`, `/resource_management/v4/resources`,
/// `/alert_manager/v1/resource_status`), all verified to share the same
/// `{"items": [...], "paging": {"cursors": {"after": "..."}}}` envelope
/// (see module docs and `parse_paged_items`). `extra_query` carries an
/// endpoint's own fixed query parameters (e.g. `("tenant_id", tenant_id)`);
/// the `after` cursor is added on top for every page after the first.
/// Stops after `MAX_PAGES` pages as protection against a misbehaving
/// remote end.
fn fetch_all_pages(
    agent: &Agent,
    url: &str,
    token: &str,
    extra_query: &[(&str, &str)],
) -> Result<Vec<serde_json::Value>, PluginError> {
    let mut all = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut request = agent
            .get(url)
            .header("Authorization", format!("Bearer {token}"));
        for (key, value) in extra_query {
            request = request.query(*key, *value);
        }
        if let Some(cursor) = after.as_deref() {
            request = request.query("after", cursor);
        }
        let mut response = request.call().map_err(map_ureq_error)?;
        let page: serde_json::Value = response.body_mut().read_json().map_err(map_ureq_error)?;
        let (items, next_after) = parse_paged_items(&page)?;
        all.extend(items);
        match next_after {
            Some(cursor) => after = Some(cursor),
            None => break,
        }
    }
    Ok(all)
}

fn fetch_all_tenants(
    agent: &Agent,
    datacenter_url: &str,
    token: &str,
    root_tenant_id: &str,
) -> Result<Vec<serde_json::Value>, PluginError> {
    let url = format!("{}/api/2/tenants", datacenter_url.trim_end_matches('/'));
    fetch_all_pages(
        agent,
        &url,
        token,
        &[("subtree_root_id", root_tenant_id), ("lod", "full")],
    )
}

/// `tenant_id_csv` is already a comma-joined subtree of tenant IDs (see
/// `fetch_tenant_subtree_csv`) -- `resource_management/v4/resources`'s
/// `tenant_id` filter is EXACT-MATCH per tenant, not recursive, verified
/// against developer.acronis.com (a real live-bug fix, see module docs).
/// A real, live-verified data-quality fix: without `is_group=false` and
/// `type=resource.machine`, this endpoint also returns dynamic/static
/// resource GROUPS ("All", "All virtual machines", "All PostgreSQL
/// databases" -- never real devices, see `is_group`/`group_condition` in
/// the module docs) and non-machine resource types (`resource.mssql_server`
/// database instances, shown as `mssql://...` entries) mixed in with real
/// machines, confirmed against a live account that showed both. Verified
/// against developer.acronis.com's real OpenAPI spec: `is_group` (query
/// param, NOT a response field) excludes groups; `type=resource.machine`
/// (the confirmed real value for an actual machine, per the same spec's
/// guide examples) excludes database/other resource types.
fn fetch_all_resources(
    agent: &Agent,
    datacenter_url: &str,
    token: &str,
    tenant_id_csv: &str,
) -> Result<Vec<serde_json::Value>, PluginError> {
    let url = format!(
        "{}/api/resource_management/v4/resources",
        datacenter_url.trim_end_matches('/')
    );
    fetch_all_pages(
        agent,
        &url,
        token,
        &[
            ("tenant_id", tenant_id_csv),
            ("is_group", "false"),
            ("type", "resource.machine"),
        ],
    )
}

/// Filters by the exact resource IDs already fetched from
/// `/resource_management/v4/resources`, NOT by tenant -- verified against
/// developer.acronis.com's real OpenAPI spec that
/// `/alert_manager/v1/resource_status` has NO `tenant`/`tenant_id`
/// parameter at all (a corrected, genuine mistake in an earlier version
/// of this module, which sent an undocumented, silently-ignored `tenant`
/// query param); its only filters are `id` (an array, using Acronis's
/// `or(id1,id2,...)` filter-expression syntax for multiple values, per
/// the same spec) and `embed_alert`. A single ID is sent plain, matching
/// the same real spec/guide-confirmed convention.
fn fetch_all_severities(
    agent: &Agent,
    datacenter_url: &str,
    token: &str,
    resource_ids: &[&str],
) -> Result<Vec<serde_json::Value>, PluginError> {
    let url = format!(
        "{}/api/alert_manager/v1/resource_status",
        datacenter_url.trim_end_matches('/')
    );
    let id_filter = if resource_ids.len() == 1 {
        resource_ids[0].to_string()
    } else {
        format!("or({})", resource_ids.join(","))
    };
    fetch_all_pages(agent, &url, token, &[("id", &id_filter)])
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Acronis-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => {
            PluginError::Unreachable(format!("Acronis-API antwortete mit Status {code}"))
        }
        ureq::Error::Json(err) => {
            PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}"))
        }
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Extracts the `items` array and the `paging.cursors.after` continuation
/// cursor from one page of Acronis's cursor-paginated response envelope
/// (`{"items": [...], "paging": {"cursors": {"after": "..."}}}`), the
/// SAME envelope shape verified for `/tenants`,
/// `/resource_management/v4/resources`, and
/// `/alert_manager/v1/resource_status` (see module docs). An absent or
/// empty `after` means "no more pages". Pure, testable with hardcoded
/// JSON, no network access needed.
fn parse_paged_items(
    json: &serde_json::Value,
) -> Result<(Vec<serde_json::Value>, Option<String>), PluginError> {
    let items = json["items"].as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse("Erwartete 'items'-Liste in Acronis-Antwort".to_string())
    })?;
    let after = json["paging"]["cursors"]["after"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok((items.clone(), after))
}

/// A single tenant object from `GET /tenants`'s `items` array
/// (`{"id", "parent_id", "name", "kind"}`, see module docs). Deliberately
/// UNFILTERED by `kind`; see `filter_customer_tenants` for that, kept as
/// its own, separately testable step. A tenant without a usable `id` or
/// `kind` is skipped: without a `kind` the customer filter downstream
/// couldn't meaningfully classify it anyway, and a single broken tenant
/// object shouldn't make the whole list unusable, analogous to
/// `plugin::ninja::map_organization`'s "skip without id" rule.
fn map_tenant(value: &serde_json::Value) -> Option<AcronisTenant> {
    let id = value["id"].as_str()?.to_string();
    let kind = value["kind"].as_str()?.to_string();
    let name = value["name"].as_str().unwrap_or(id.as_str()).to_string();
    Some(AcronisTenant { id, name, kind })
}

/// Narrows a tenant list down to `kind == "customer"`, the only valid
/// mapping targets (see module docs: `root`/`partner`/`folder`/`unit` are
/// organizational containers, not real customer accounts). Deliberately a
/// separate, pure function from `map_tenant` so both the raw mapping and
/// the filter are independently unit-testable (see brief/module docs).
fn filter_customer_tenants(tenants: &[AcronisTenant]) -> Vec<AcronisTenant> {
    tenants
        .iter()
        .filter(|t| t.kind == "customer")
        .cloned()
        .collect()
}

/// Maximum tenant IDs sent in one comma-joined `tenant_id` filter value on
/// `/resource_management/v4/resources`/`resource_statuses`, per that
/// endpoint's own documented "Maximum 100 items" limit (verified against
/// developer.acronis.com's OpenAPI spec). A single mapped customer having
/// more than 100 descendant tenants/units is not a realistic case for this
/// app's single-admin scale; truncating rather than paginating multiple
/// requests keeps this simple, matching every other defensive cap already
/// used in this module (`MAX_PAGES`).
const MAX_SUBTREE_TENANT_IDS: usize = 100;

/// Resolves every tenant ID in `root_id`'s subtree (itself plus all
/// transitive children by `parent_id`) from a raw, UNFILTERED `/tenants`
/// page collection (see module docs: a genuine fix for a real live bug --
/// `resource_management/v4/resources`'s `tenant_id` filter is EXACT-MATCH
/// only, verified against developer.acronis.com, NOT recursive into child
/// tenants/units the way Account Management's `subtree_root_id` is. A
/// mapped "customer" tenant can have resources actually registered under
/// its own child units/sites, invisible if only the customer tenant's own
/// ID is queried -- this function computes the full descendant set so the
/// resources call can pass all of them). Pure, testable with hardcoded
/// JSON. A tenant entry missing a usable `id` is skipped (matches
/// `map_tenant`'s own tolerance); an entry whose `parent_id` points at an
/// ID not present in `raw_tenants` (e.g. a page boundary or an
/// inaccessible ancestor) simply never gets visited as a child, it does
/// not panic or error. Capped at `MAX_SUBTREE_TENANT_IDS` entries.
fn collect_subtree_tenant_ids(raw_tenants: &[serde_json::Value], root_id: &str) -> Vec<String> {
    let mut children_by_parent: HashMap<&str, Vec<&str>> = HashMap::new();
    for value in raw_tenants {
        let (Some(id), Some(parent_id)) = (value["id"].as_str(), value["parent_id"].as_str())
        else {
            continue;
        };
        children_by_parent.entry(parent_id).or_default().push(id);
    }

    let mut result = vec![root_id.to_string()];
    let mut queue: Vec<&str> = vec![root_id];
    while let Some(current) = queue.pop() {
        if result.len() >= MAX_SUBTREE_TENANT_IDS {
            break;
        }
        if let Some(children) = children_by_parent.get(current) {
            for &child in children {
                if result.len() >= MAX_SUBTREE_TENANT_IDS {
                    break;
                }
                result.push(child.to_string());
                queue.push(child);
            }
        }
    }
    result
}

/// A real, live-verified fix: the same physical/virtual machine can be
/// reported as multiple separate resource entries (different agents/
/// backup roles protecting it -- see `aggregate_id`/`aggregation_status`/
/// `aspects` in Acronis's own Resource schema, confirmed against
/// developer.acronis.com), which showed up as e.g. "SRV-LF-SU-20" and
/// "SRV-LF-SU-20.lflohmar.local" as two separate rows against a real
/// account. Keeps only the FIRST resource seen per `aggregate_id`; a
/// resource with no `aggregate_id` at all falls back to its own `id` as
/// the dedup key (so it never collides with an unrelated resource that
/// also happens to lack one). Order-preserving (first occurrence wins),
/// pure and testable with hardcoded JSON, no network access.
fn dedupe_resources_by_aggregate_id(
    raw_resources: Vec<serde_json::Value>,
) -> Vec<serde_json::Value> {
    let mut seen_keys = std::collections::HashSet::new();
    let mut result = Vec::new();
    for value in raw_resources {
        let key = value["aggregate_id"]
            .as_str()
            .map(str::to_string)
            .or_else(|| value["id"].as_str().map(str::to_string));
        let Some(key) = key else {
            // No aggregate_id AND no id -- map_resource will skip this
            // entry anyway (it requires a usable `id`), so let it through
            // unchanged rather than silently dropping it here too.
            result.push(value);
            continue;
        };
        if seen_keys.insert(key) {
            result.push(value);
        }
    }
    result
}

/// A single resource object from
/// `GET /resource_management/v4/resources`'s `items` array (`{"id",
/// "name", "agent_id", "external_id", "type"}`, see module docs). `id`
/// becomes `AcronisResource::external_id` (NOT the resource's own,
/// confusingly-named `external_id` field, and NOT `agent_id`; see module
/// docs on why). `backup_status` always starts `None` here, filled in
/// later by `join_resources_with_severity`. A resource without a usable
/// `id` is skipped, same "don't fail the whole call over one broken
/// entry" convention as every other plugin's mapping function here.
fn map_resource(
    value: &serde_json::Value,
    tenant_id: &str,
    tenant_name: &str,
) -> Option<AcronisResource> {
    let external_id = value["id"].as_str()?.to_string();
    let name = value["name"]
        .as_str()
        .unwrap_or(external_id.as_str())
        .to_string();
    Some(AcronisResource {
        external_id,
        name,
        tenant_id: tenant_id.to_string(),
        tenant_name: tenant_name.to_string(),
        backup_status: None,
    })
}

/// Maps `GET /alert_manager/v1/resource_status`'s raw `items` array
/// (`{"id": "<resourceId>", "severity": "...", "alert": {...}}`, see
/// module docs) into a lookup table `resource id -> severity`, for
/// `join_resources_with_severity` below. An entry without a usable
/// `id`/`severity` is skipped.
fn map_severities_response(items: &[serde_json::Value]) -> HashMap<String, String> {
    items
        .iter()
        .filter_map(|v| {
            let id = v["id"].as_str()?.to_string();
            let severity = v["severity"].as_str()?.to_string();
            Some((id, severity))
        })
        .collect()
}

/// The trickiest logic in this plugin (see module docs and the brief this
/// plugin was built from): joins each resource's own `external_id` (==
/// Acronis's resource `id`, see `map_resource`) against the alert-manager
/// severity lookup (`map_severities_response`), setting
/// `AcronisResource::backup_status`. A resource with NO matching entry
/// (never backed up / not protected) keeps `backup_status: None`, a
/// legitimate, expected case, not an error. Pure function, given thorough
/// test coverage below (matching id, no matching alert entry, empty alert
/// list) per the brief this plugin was built from.
fn join_resources_with_severity(
    resources: Vec<AcronisResource>,
    severities: &HashMap<String, String>,
) -> Vec<AcronisResource> {
    resources
        .into_iter()
        .map(|mut resource| {
            resource.backup_status = severities.get(&resource.external_id).cloned();
            resource
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_a_valid_tenant() {
        let value = serde_json::json!({
            "id": "tenant-1",
            "parent_id": "root-1",
            "name": "Acme Corp",
            "kind": "customer"
        });
        let tenant = map_tenant(&value).unwrap();
        assert_eq!(tenant.id, "tenant-1");
        assert_eq!(tenant.name, "Acme Corp");
        assert_eq!(tenant.kind, "customer");
    }

    #[test]
    fn tenant_falls_back_to_id_when_no_name_field_present() {
        let value = serde_json::json!({"id": "tenant-9", "kind": "unit"});
        let tenant = map_tenant(&value).unwrap();
        assert_eq!(tenant.name, "tenant-9");
    }

    #[test]
    fn tenant_skips_entries_without_a_usable_id() {
        let value = serde_json::json!({"name": "OhneId", "kind": "customer"});
        assert!(map_tenant(&value).is_none());
    }

    #[test]
    fn tenant_skips_entries_without_a_usable_kind() {
        let value = serde_json::json!({"id": "tenant-1", "name": "Acme"});
        assert!(map_tenant(&value).is_none());
    }

    fn tenant(id: &str, kind: &str) -> AcronisTenant {
        AcronisTenant {
            id: id.to_string(),
            name: format!("Tenant {id}"),
            kind: kind.to_string(),
        }
    }

    #[test]
    fn filter_customer_tenants_keeps_only_customer_kind() {
        let tenants = vec![
            tenant("1", "root"),
            tenant("2", "partner"),
            tenant("3", "folder"),
            tenant("4", "customer"),
            tenant("5", "unit"),
            tenant("6", "customer"),
        ];
        let filtered = filter_customer_tenants(&tenants);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|t| t.kind == "customer"));
        assert_eq!(filtered[0].id, "4");
        assert_eq!(filtered[1].id, "6");
    }

    #[test]
    fn filter_customer_tenants_returns_empty_for_no_customer_kind_tenants() {
        let tenants = vec![tenant("1", "root"), tenant("2", "partner")];
        assert!(filter_customer_tenants(&tenants).is_empty());
    }

    fn raw_tenant(id: &str, parent_id: &str) -> serde_json::Value {
        serde_json::json!({"id": id, "parent_id": parent_id, "name": id, "kind": "unit"})
    }

    #[test]
    fn subtree_of_a_leaf_tenant_with_no_children_is_just_itself() {
        let raw = vec![raw_tenant("customer-1", "root")];
        let ids = collect_subtree_tenant_ids(&raw, "customer-1");
        assert_eq!(ids, vec!["customer-1"]);
    }

    #[test]
    fn subtree_walks_a_linear_chain_of_descendants() {
        let raw = vec![
            raw_tenant("customer-1", "root"),
            raw_tenant("site-a", "customer-1"),
            raw_tenant("unit-a1", "site-a"),
        ];
        let mut ids = collect_subtree_tenant_ids(&raw, "customer-1");
        ids.sort();
        assert_eq!(ids, vec!["customer-1", "site-a", "unit-a1"]);
    }

    #[test]
    fn subtree_walks_a_branching_tree_of_descendants() {
        let raw = vec![
            raw_tenant("customer-1", "root"),
            raw_tenant("site-a", "customer-1"),
            raw_tenant("site-b", "customer-1"),
            raw_tenant("unit-a1", "site-a"),
        ];
        let mut ids = collect_subtree_tenant_ids(&raw, "customer-1");
        ids.sort();
        assert_eq!(ids, vec!["customer-1", "site-a", "site-b", "unit-a1"]);
    }

    #[test]
    fn subtree_excludes_tenants_outside_the_requested_root() {
        let raw = vec![
            raw_tenant("customer-1", "root"),
            raw_tenant("site-a", "customer-1"),
            raw_tenant("customer-2", "root"),
            raw_tenant("site-x", "customer-2"),
        ];
        let mut ids = collect_subtree_tenant_ids(&raw, "customer-1");
        ids.sort();
        assert_eq!(ids, vec!["customer-1", "site-a"]);
    }

    #[test]
    fn subtree_skips_entries_missing_id_or_parent_id_without_panicking() {
        let raw = vec![
            raw_tenant("customer-1", "root"),
            serde_json::json!({"name": "broken", "kind": "unit"}),
            raw_tenant("site-a", "customer-1"),
        ];
        let mut ids = collect_subtree_tenant_ids(&raw, "customer-1");
        ids.sort();
        assert_eq!(ids, vec!["customer-1", "site-a"]);
    }

    #[test]
    fn subtree_is_capped_at_max_subtree_tenant_ids() {
        let mut raw = Vec::new();
        for i in 0..150 {
            raw.push(raw_tenant(&format!("child-{i}"), "customer-1"));
        }
        let ids = collect_subtree_tenant_ids(&raw, "customer-1");
        assert_eq!(ids.len(), MAX_SUBTREE_TENANT_IDS);
        assert!(ids.contains(&"customer-1".to_string()));
    }

    #[test]
    fn maps_a_valid_resource() {
        let value = serde_json::json!({
            "id": "res-1",
            "name": "SRV-01",
            "agent_id": "agent-abc",
            "external_id": "some-other-external-id",
            "type": "machine"
        });
        let resource = map_resource(&value, "tenant-1", "Acme Corp").unwrap();
        // The resource's own "id" becomes external_id, NOT its own
        // "external_id" field, see module docs.
        assert_eq!(resource.external_id, "res-1");
        assert_eq!(resource.name, "SRV-01");
        assert_eq!(resource.tenant_id, "tenant-1");
        assert_eq!(resource.tenant_name, "Acme Corp");
        assert_eq!(resource.backup_status, None);
    }

    #[test]
    fn resource_falls_back_to_id_when_no_name_field_present() {
        let value = serde_json::json!({"id": "res-7"});
        let resource = map_resource(&value, "tenant-1", "Acme").unwrap();
        assert_eq!(resource.name, "res-7");
    }

    #[test]
    fn resource_skips_entries_without_a_usable_id() {
        let value = serde_json::json!({"name": "OhneId"});
        assert!(map_resource(&value, "tenant-1", "Acme").is_none());
    }

    #[test]
    fn dedupe_keeps_only_the_first_resource_per_aggregate_id() {
        let raw = vec![
            serde_json::json!({"id": "res-1", "name": "SRV-LF-SU-20", "aggregate_id": "agg-1"}),
            serde_json::json!({"id": "res-2", "name": "SRV-LF-SU-20.lflohmar.local", "aggregate_id": "agg-1"}),
        ];
        let deduped = dedupe_resources_by_aggregate_id(raw);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0]["id"], "res-1");
    }

    #[test]
    fn dedupe_falls_back_to_id_when_aggregate_id_is_absent() {
        let raw = vec![
            serde_json::json!({"id": "res-1", "name": "webportal"}),
            serde_json::json!({"id": "res-2", "name": "databaseserver"}),
        ];
        let deduped = dedupe_resources_by_aggregate_id(raw);
        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn dedupe_does_not_collide_resources_that_all_lack_an_id() {
        let raw = vec![
            serde_json::json!({"name": "OhneId1"}),
            serde_json::json!({"name": "OhneId2"}),
        ];
        let deduped = dedupe_resources_by_aggregate_id(raw);
        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn maps_severities_into_a_lookup_table() {
        let items = vec![
            serde_json::json!({"id": "res-1", "severity": "ok", "alert": {}}),
            serde_json::json!({"id": "res-2", "severity": "critical", "alert": {}}),
        ];
        let map = map_severities_response(&items);
        assert_eq!(map.get("res-1").map(String::as_str), Some("ok"));
        assert_eq!(map.get("res-2").map(String::as_str), Some("critical"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn severities_skip_entries_without_id_or_severity() {
        let items = vec![
            serde_json::json!({"severity": "ok"}),
            serde_json::json!({"id": "res-3"}),
        ];
        let map = map_severities_response(&items);
        assert!(map.is_empty());
    }

    fn resource(external_id: &str) -> AcronisResource {
        AcronisResource {
            external_id: external_id.to_string(),
            name: format!("Resource {external_id}"),
            tenant_id: "tenant-1".to_string(),
            tenant_name: "Acme".to_string(),
            backup_status: None,
        }
    }

    #[test]
    fn join_sets_backup_status_for_a_matching_resource_id() {
        let resources = vec![resource("res-1")];
        let mut severities = HashMap::new();
        severities.insert("res-1".to_string(), "warning".to_string());

        let joined = join_resources_with_severity(resources, &severities);

        assert_eq!(joined[0].backup_status.as_deref(), Some("warning"));
    }

    #[test]
    fn join_leaves_backup_status_none_when_no_matching_alert_entry() {
        let resources = vec![resource("res-1"), resource("res-2")];
        let mut severities = HashMap::new();
        severities.insert("res-2".to_string(), "critical".to_string());

        let joined = join_resources_with_severity(resources, &severities);

        assert_eq!(joined[0].external_id, "res-1");
        assert_eq!(joined[0].backup_status, None);
        assert_eq!(joined[1].backup_status.as_deref(), Some("critical"));
    }

    #[test]
    fn join_leaves_every_resource_none_with_an_empty_severity_list() {
        let resources = vec![resource("res-1"), resource("res-2")];
        let severities = HashMap::new();

        let joined = join_resources_with_severity(resources, &severities);

        assert!(joined.iter().all(|r| r.backup_status.is_none()));
    }

    #[test]
    fn parses_paged_items_with_after_cursor() {
        let json = serde_json::json!({
            "items": [{"id": "a"}, {"id": "b"}],
            "paging": {"cursors": {"after": "cursor-2"}}
        });
        let (items, after) = parse_paged_items(&json).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(after.as_deref(), Some("cursor-2"));
    }

    #[test]
    fn parses_paged_items_without_after_cursor_as_last_page() {
        let json = serde_json::json!({"items": [], "paging": {"cursors": {}}});
        let (items, after) = parse_paged_items(&json).unwrap();
        assert!(items.is_empty());
        assert_eq!(after, None);
    }

    #[test]
    fn parses_paged_items_treats_empty_after_as_no_more_pages() {
        let json = serde_json::json!({"items": [], "paging": {"cursors": {"after": ""}}});
        let (_, after) = parse_paged_items(&json).unwrap();
        assert_eq!(after, None);
    }

    #[test]
    fn parse_paged_items_rejects_missing_items_field() {
        let json = serde_json::json!({"paging": {"cursors": {}}});
        let result = parse_paged_items(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn parses_credentials_from_json_secret() {
        let creds = PluginCredentials {
            secret: r#"{"client_id":"cid","client_secret":"sec"}"#.into(),
        };
        let parsed = parse_credentials(&creds).unwrap();
        assert_eq!(parsed.client_id, "cid");
        assert_eq!(parsed.client_secret, "sec");
    }

    #[test]
    fn rejects_malformed_credentials_json() {
        let creds = PluginCredentials {
            secret: "not json".into(),
        };
        let result = parse_credentials(&creds);
        assert!(matches!(result, Err(PluginError::Authentication(_))));
    }

    #[test]
    fn maps_401_status_to_authentication_error() {
        let err = map_ureq_error(ureq::Error::StatusCode(401));
        assert!(matches!(err, PluginError::Authentication(_)));
    }

    #[test]
    fn maps_403_status_to_authentication_error() {
        let err = map_ureq_error(ureq::Error::StatusCode(403));
        assert!(matches!(err, PluginError::Authentication(_)));
    }

    #[test]
    fn maps_other_status_codes_to_unreachable_error() {
        let err = map_ureq_error(ureq::Error::StatusCode(500));
        assert!(matches!(err, PluginError::Unreachable(_)));
    }

    #[test]
    fn plugin_id_returns_configured_connection_id() {
        let plugin = AcronisPlugin::new(
            "acronis:acme-123".to_string(),
            "https://eu2-cloud.acronis.com".to_string(),
        );
        assert_eq!(plugin.id(), "acronis:acme-123");
    }

    #[test]
    fn list_systems_is_deliberately_empty() {
        let plugin = AcronisPlugin::new(
            "acronis:acme-123".to_string(),
            "https://eu2-cloud.acronis.com".to_string(),
        );
        let creds = PluginCredentials {
            secret: r#"{"client_id":"cid","client_secret":"sec"}"#.into(),
        };
        let result = plugin.list_systems(&creds).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn get_system_details_trait_method_reports_the_honest_gap() {
        let plugin = AcronisPlugin::new(
            "acronis:acme-123".to_string(),
            "https://eu2-cloud.acronis.com".to_string(),
        );
        let creds = PluginCredentials {
            secret: r#"{"client_id":"cid","client_secret":"sec"}"#.into(),
        };
        let result = plugin.get_system_details(&creds, "res-1");
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn link_system_never_fails() {
        let plugin = AcronisPlugin::new(
            "acronis:acme-123".to_string(),
            "https://eu2-cloud.acronis.com".to_string(),
        );
        assert!(plugin.link_system(1, "res-1").is_ok());
    }
}
