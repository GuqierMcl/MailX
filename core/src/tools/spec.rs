//! Tool metadata, the registry, and the single call path every transport uses.
use std::cell::RefCell;
use std::sync::Arc;

use schemars::generate::SchemaSettings;
use schemars::transform::{Transform, transform_subschemas};
use schemars::{JsonSchema, Schema};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::audit::{NoAudit, ToolAudit, ToolCallRecord};
use super::context::{Action, Audience, RequestContext};
use super::error::{ToolError, ToolErrorCode};
use crate::MailxCore;

/// Per-audience limits. Kept as separate values per audience so one
/// surface can be tightened without touching the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AudiencePolicy {
    pub default_page_size: u32,
    pub max_page_size: u32,
    /// Unicode characters of body text per `read_email` call.
    pub default_text_chars: usize,
    pub max_text_chars: usize,
}

impl AudiencePolicy {
    pub const DEFAULT: Self = Self {
        default_page_size: 20,
        max_page_size: 100,
        default_text_chars: 8_000,
        max_text_chars: 32_000,
    };
}

/// Behaviour hints for clients; never a security control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotations {
    pub read_only: bool,
    /// Writes MailX's local data (never mail servers).
    pub writes_local_data: bool,
}

impl Annotations {
    pub const READ_ONLY: Self = Self {
        read_only: true,
        writes_local_data: false,
    };
    pub const WRITES_LOCAL: Self = Self {
        read_only: false,
        writes_local_data: true,
    };
}

#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub required_actions: Vec<Action>,
    /// Audiences that can see and call the tool; none by default.
    pub visibility: Vec<(Audience, AudiencePolicy)>,
    pub annotations: Annotations,
    pub input_schema: Value,
    pub output_schema: Value,
}

impl ToolSpec {
    /// Schemas come from the input and output types.
    pub fn new<I: JsonSchema, O: JsonSchema>(
        id: &'static str,
        title: &'static str,
        description: &'static str,
        annotations: Annotations,
    ) -> Self {
        Self {
            id,
            title,
            description,
            required_actions: Vec::new(),
            visibility: Vec::new(),
            annotations,
            input_schema: schema_of::<I>(),
            output_schema: schema_of::<O>(),
        }
    }

    pub fn requires(mut self, actions: &[Action]) -> Self {
        self.required_actions = actions.to_vec();
        self
    }

    pub fn visible_to(mut self, audience: Audience, policy: AudiencePolicy) -> Self {
        self.visibility.retain(|(a, _)| *a != audience);
        self.visibility.push((audience, policy));
        self
    }

    fn policy_for(&self, audience: Audience) -> Option<AudiencePolicy> {
        self.visibility
            .iter()
            .find(|(a, _)| *a == audience)
            .map(|(_, p)| *p)
    }
}

/// The JSON Schema advertised for a tool input or output.
///
/// Optional values are omittable, never nullable: schemars renders
/// `Option<T>` as `"type": ["string", "null"]`, which not every MCP client
/// accepts. Optional fields are not `required` and carry a single type; an
/// output field whose absence means "none" must skip serialization when empty.
fn schema_of<T: JsonSchema>() -> Value {
    let generator = SchemaSettings::draft2020_12()
        .with_transform(DropNull)
        .into_generator();
    serde_json::to_value(generator.into_root_schema_for::<T>()).unwrap_or(Value::Null)
}

/// Removes the `null` alternative that schemars adds for `Option<T>`, whether
/// it is a `type` array, an `anyOf`/`oneOf` branch, or an `enum` value.
#[derive(Clone)]
struct DropNull;

impl Transform for DropNull {
    fn transform(&mut self, schema: &mut Schema) {
        transform_subschemas(self, schema);
        let Some(object) = schema.as_object_mut() else {
            return;
        };
        if let Some(Value::Array(types)) = object.get_mut("type") {
            types.retain(|t| t != "null");
            if types.len() == 1 {
                let only = types.remove(0);
                object.insert("type".into(), only);
            }
        }
        if let Some(Value::Array(values)) = object.get_mut("enum") {
            values.retain(|v| !v.is_null());
        }
        for key in ["anyOf", "oneOf"] {
            let Some(Value::Array(branches)) = object.get_mut(key) else {
                continue;
            };
            branches.retain(|b| b.get("type") != Some(&Value::from("null")));
            if branches.len() == 1 {
                let Value::Object(inner) = branches.remove(0) else {
                    continue;
                };
                object.remove(key);
                for (k, v) in inner {
                    object.entry(k).or_insert(v);
                }
            }
        }
    }
}

/// What a transport advertises for one tool.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDescriptor {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub output_schema: Value,
    pub annotations: Annotations,
    pub required_actions: Vec<Action>,
    /// Limits that apply to the audience this descriptor was listed for.
    pub limits: AudiencePolicy,
}

/// Handler-side view of one authorized call.
pub struct ToolCall<'a> {
    pub core: &'a MailxCore,
    pub ctx: &'a RequestContext,
    pub policy: AudiencePolicy,
    default_account: &'a (dyn Fn() -> String + Send + Sync),
    resources: RefCell<Vec<String>>,
}

impl ToolCall<'_> {
    /// The host's default account id (may be empty when none is set).
    pub fn default_account_id(&self) -> String {
        (self.default_account)()
    }

    /// Records an account or email id for the call log.
    pub fn note_resource(&self, id: &str) {
        let mut resources = self.resources.borrow_mut();
        if resources.len() < 32 && !resources.iter().any(|r| r == id) {
            resources.push(id.chars().take(64).collect());
        }
    }

    /// Fails like a missing resource when the account is out of scope.
    pub fn require_account(&self, account_id: &str) -> Result<(), ToolError> {
        if self.ctx.grant.accounts.permits(account_id) {
            Ok(())
        } else {
            Err(ToolError::new(ToolErrorCode::NotFound))
        }
    }
}

type Erased = Box<dyn Fn(&ToolCall<'_>, Value) -> Result<Value, ToolError> + Send + Sync>;

struct Entry {
    spec: ToolSpec,
    run: Erased,
}

/// The tool registry shared by every host. Calls are synchronous and block
/// on storage like [`MailxCore`]; async hosts must use `spawn_blocking`.
pub struct Tools {
    core: MailxCore,
    default_account: Arc<dyn Fn() -> String + Send + Sync>,
    audit: Arc<dyn ToolAudit>,
    entries: Vec<Entry>,
}

impl Tools {
    /// `default_account` reads the host's `compose.defaultAccountId` at call
    /// time. Registers the built-in tools.
    pub fn new(
        core: MailxCore,
        default_account: impl Fn() -> String + Send + Sync + 'static,
    ) -> Self {
        let mut tools = Self {
            core,
            default_account: Arc::new(default_account),
            audit: Arc::new(NoAudit),
            entries: Vec::new(),
        };
        super::accounts::register(&mut tools);
        super::mail::register(&mut tools);
        super::drafts::register(&mut tools);
        tools
    }

    /// Sends one record per call to `audit`.
    pub fn with_audit(mut self, audit: Arc<dyn ToolAudit>) -> Self {
        self.audit = audit;
        self
    }

    /// Adds or replaces a tool. Handlers get typed, already validated JSON
    /// input; they must enforce account scope through [`ToolCall`].
    pub fn register<I, O>(
        &mut self,
        spec: ToolSpec,
        handler: impl Fn(&ToolCall<'_>, I) -> Result<O, ToolError> + Send + Sync + 'static,
    ) where
        I: DeserializeOwned,
        O: Serialize,
    {
        let run: Erased = Box::new(move |call, input| {
            let input: I = serde_json::from_value(input).map_err(|_| {
                ToolError::invalid("input", "does not match the tool's input schema")
            })?;
            let output = handler(call, input)?;
            serde_json::to_value(output).map_err(|_| ToolError::new(ToolErrorCode::Internal))
        });
        self.entries.retain(|e| e.spec.id != spec.id);
        self.entries.push(Entry { spec, run });
    }

    /// Tools the audience can see, in registration order.
    pub fn list(&self, audience: Audience) -> Vec<ToolDescriptor> {
        self.entries
            .iter()
            .filter_map(|e| describe(&e.spec, audience))
            .collect()
    }

    /// Like [`Tools::list`], also hiding tools the grant cannot use.
    pub fn list_for(&self, ctx: &RequestContext) -> Vec<ToolDescriptor> {
        self.entries
            .iter()
            .filter(|e| e.spec.required_actions.iter().all(|a| ctx.grant.allows(*a)))
            .filter_map(|e| describe(&e.spec, ctx.principal.audience()))
            .collect()
    }

    /// Runs one tool. Visibility, actions and input are checked on every
    /// call; nothing about authorization is cached.
    pub fn call(
        &self,
        ctx: &RequestContext,
        tool_id: &str,
        input: Value,
    ) -> Result<Value, ToolError> {
        let (result, resources) = self.dispatch(ctx, tool_id, input);
        self.audit.record(&ToolCallRecord {
            request_id: ctx.request_id.clone(),
            principal_kind: ctx.principal.kind(),
            principal_id: ctx.principal.id().to_owned(),
            client_name: match &ctx.principal {
                super::Principal::LocalMcp { client_name, .. } => {
                    client_name.as_ref().map(|n| n.chars().take(64).collect())
                }
                _ => None,
            },
            tool_id: tool_id.chars().take(64).collect(),
            resource_ids: resources,
            error: result.as_ref().err().map(|e| e.code),
        });
        result
    }

    fn dispatch(
        &self,
        ctx: &RequestContext,
        tool_id: &str,
        input: Value,
    ) -> (Result<Value, ToolError>, Vec<String>) {
        let audience = ctx.principal.audience();
        let Some((entry, policy)) = self
            .entries
            .iter()
            .find(|e| e.spec.id == tool_id)
            .and_then(|e| e.spec.policy_for(audience).map(|p| (e, p)))
        else {
            return (Err(ToolError::new(ToolErrorCode::UnknownTool)), Vec::new());
        };
        if !entry
            .spec
            .required_actions
            .iter()
            .all(|a| ctx.grant.allows(*a))
        {
            return (Err(ToolError::new(ToolErrorCode::NotPermitted)), Vec::new());
        }
        // Hosts may send null for a tool without arguments.
        let input = if input.is_null() {
            Value::Object(Default::default())
        } else {
            input
        };
        let call = ToolCall {
            core: &self.core,
            ctx,
            policy,
            default_account: &*self.default_account,
            resources: RefCell::new(Vec::new()),
        };
        let result = (entry.run)(&call, input);
        (result, call.resources.into_inner())
    }
}

fn describe(spec: &ToolSpec, audience: Audience) -> Option<ToolDescriptor> {
    let limits = spec.policy_for(audience)?;
    Some(ToolDescriptor {
        id: spec.id,
        title: spec.title,
        description: spec.description,
        input_schema: spec.input_schema.clone(),
        output_schema: spec.output_schema.clone(),
        annotations: spec.annotations,
        required_actions: spec.required_actions.clone(),
        limits,
    })
}
