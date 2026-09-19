use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::tool::ToolProgressSender;
use crate::agent::{Backend as LlmProvider, Tool, ToolContext, ToolResult};
use crate::cdp::PageSession;
use crate::sites::registry::{
    required_string, ArgKind, BoxFuture, CommandArg, NativeSiteAdapter, SiteCommand, SlowWhen,
};
use crate::sites::runner::{get_f64, get_i64, get_str, json_result, ToolCommand};
use crate::sites::skill_cli::{
    current_url, ensure_site_page, failure_payload, gate_reason, invoke_browser_tool,
    navigate_https, percent_encode_query, run_skill_command, wait_for_browser_tool,
    DEFAULT_COMMENT_COUNT, DEFAULT_RESULT_COUNT, DEFAULT_WAIT_SECONDS, MAX_TOOL_ITEMS,
    MAX_TOOL_WAIT_SECONDS,
};

const SITE_ID: &str = "linkedin";
const HOME_URL: &str = "https://www.linkedin.com/";
const HOST_ROOT: &str = "linkedin.com";

pub async fn linkedin_agent_tools(
    page: Arc<PageSession>,
    _llm_provider: Arc<dyn LlmProvider>,
) -> anyhow::Result<Vec<Arc<dyn Tool>>> {
    Ok(linkedin_tools(page))
}

pub fn linkedin_agent_instructions(extra: &str) -> String {
    crate::sites::learning::site_agent_instructions(SITE_ID, extra)
}

fn linkedin_tools(page: Arc<PageSession>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(SearchTool { page: page.clone() }),
        Arc::new(ProfileTool { page: page.clone() }),
        Arc::new(HistoryTool { page: page.clone() }),
        Arc::new(CompanyTool { page: page.clone() }),
        Arc::new(CompanyPeopleTool { page: page.clone() }),
        Arc::new(RelatedPeopleTool { page: page.clone() }),
        Arc::new(GetPostsTool { page: page.clone() }),
        Arc::new(PageStateTool { page }),
    ]
}

pub static LINKEDIN_NATIVE_ADAPTER: NativeSiteAdapter = NativeSiteAdapter {
    id: SITE_ID,
    about: "LinkedIn (linkedin.com)",
    home_url: "",
    agent_tools: |page, llm| Box::pin(linkedin_agent_tools(page, llm)),
    default_agent_tools: None,
    agent_instructions: linkedin_agent_instructions,
    default_agent_instructions: None,
    commands: &[
        SiteCommand {
            name: "search",
            tool_name: "search",
            about: "Search LinkedIn people, companies, or content and print result cards as JSON.",
            args: &[
                CommandArg {
                    key: "query",
                    long: None,
                    value_name: "QUERY",
                    help: "Search query",
                    required: true,
                    kind: ArgKind::Str,
                },
                CommandArg {
                    key: "result_type",
                    long: Some("type"),
                    value_name: "TYPE",
                    help: "Result type: people, content, companies, or all. Defaults to people.",
                    required: false,
                    kind: ArgKind::Str,
                },
                CommandArg {
                    key: "num",
                    long: Some("num"),
                    value_name: "N",
                    help: "Number of results to collect by scrolling. Defaults to 10.",
                    required: false,
                    kind: ArgKind::Int,
                },
                CommandArg {
                    key: "wait_seconds",
                    long: Some("wait-seconds"),
                    value_name: "SECONDS",
                    help: "Maximum wait for the search page to hydrate. Defaults to 30.",
                    required: false,
                    kind: ArgKind::Int,
                },
            ],
            slow: SlowWhen::Always,
            run: run_search,
        },
        SiteCommand {
            name: "profile",
            tool_name: "profile",
            about: "Read a LinkedIn profile landing page.",
            args: &[
                CommandArg {
                    key: "profile",
                    long: None,
                    value_name: "ID_OR_URL",
                    help: "LinkedIn profile id or /in/ URL.",
                    required: true,
                    kind: ArgKind::Str,
                },
                CommandArg {
                    key: "wait_seconds",
                    long: Some("wait-seconds"),
                    value_name: "SECONDS",
                    help: "Maximum wait for the profile page to hydrate. Defaults to 30.",
                    required: false,
                    kind: ArgKind::Int,
                },
            ],
            slow: SlowWhen::Always,
            run: run_profile,
        },
        SiteCommand {
            name: "history",
            tool_name: "history",
            about: "Read complete visible work or education history from a LinkedIn profile details route.",
            args: &[
                CommandArg {
                    key: "profile",
                    long: None,
                    value_name: "ID_OR_URL",
                    help: "LinkedIn profile id or /in/ URL.",
                    required: true,
                    kind: ArgKind::Str,
                },
                CommandArg {
                    key: "section",
                    long: Some("section"),
                    value_name: "SECTION",
                    help: "History section: experience or education. Defaults to experience.",
                    required: false,
                    kind: ArgKind::Str,
                },
                CommandArg {
                    key: "wait_seconds",
                    long: Some("wait-seconds"),
                    value_name: "SECONDS",
                    help: "Maximum wait for the details page to hydrate. Defaults to 30.",
                    required: false,
                    kind: ArgKind::Int,
                },
            ],
            slow: SlowWhen::Always,
            run: run_history,
        },
        SiteCommand {
            name: "company",
            tool_name: "company",
            about: "Read a LinkedIn company or showcase landing page.",
            args: &[
                CommandArg {
                    key: "company",
                    long: None,
                    value_name: "ID_OR_URL",
                    help: "LinkedIn company id or /company/ or /showcase/ URL.",
                    required: true,
                    kind: ArgKind::Str,
                },
                CommandArg {
                    key: "wait_seconds",
                    long: Some("wait-seconds"),
                    value_name: "SECONDS",
                    help: "Maximum wait for the company page to hydrate. Defaults to 30.",
                    required: false,
                    kind: ArgKind::Int,
                },
            ],
            slow: SlowWhen::Always,
            run: run_company,
        },
        SiteCommand {
            name: "company-people",
            tool_name: "company_people",
            about: "Read the visible People you may know suggestions on a LinkedIn company people page.",
            args: &[
                CommandArg {
                    key: "company",
                    long: None,
                    value_name: "ID_OR_URL",
                    help: "LinkedIn company id or company/people URL.",
                    required: true,
                    kind: ArgKind::Str,
                },
                CommandArg {
                    key: "num",
                    long: Some("num"),
                    value_name: "N",
                    help: "Maximum visible profile suggestions to return. Defaults to 10.",
                    required: false,
                    kind: ArgKind::Int,
                },
                CommandArg {
                    key: "wait_seconds",
                    long: Some("wait-seconds"),
                    value_name: "SECONDS",
                    help: "Maximum wait for the company people page to hydrate. Defaults to 30.",
                    required: false,
                    kind: ArgKind::Int,
                },
            ],
            slow: SlowWhen::Always,
            run: run_company_people,
        },
        SiteCommand {
            name: "related-people",
            tool_name: "related_people",
            about: "Read explicitly labelled related-people sections on a LinkedIn profile or company page.",
            args: &[
                CommandArg {
                    key: "target",
                    long: None,
                    value_name: "ID_OR_URL",
                    help: "LinkedIn profile id, company id, or page URL.",
                    required: true,
                    kind: ArgKind::Str,
                },
                CommandArg {
                    key: "num",
                    long: Some("num"),
                    value_name: "N",
                    help: "Maximum visible related profiles to return. Defaults to 10.",
                    required: false,
                    kind: ArgKind::Int,
                },
                CommandArg {
                    key: "wait_seconds",
                    long: Some("wait-seconds"),
                    value_name: "SECONDS",
                    help: "Maximum wait for the page to hydrate. Defaults to 30.",
                    required: false,
                    kind: ArgKind::Int,
                },
            ],
            slow: SlowWhen::Always,
            run: run_related_people,
        },
        SiteCommand {
            name: "get-posts",
            tool_name: "get_posts",
            about: "Read LinkedIn posts by URL, including comments.",
            args: &[
                CommandArg {
                    key: "posts",
                    long: Some("post"),
                    value_name: "URL",
                    help: "LinkedIn post or feed-update URL. Repeat to read multiple posts.",
                    required: true,
                    kind: ArgKind::StrList,
                },
                CommandArg {
                    key: "num_comments",
                    long: Some("num-comments"),
                    value_name: "N",
                    help: "Comments to collect per post. Defaults to 8; 0 skips comments.",
                    required: false,
                    kind: ArgKind::Int,
                },
                CommandArg {
                    key: "wait_seconds",
                    long: Some("wait-seconds"),
                    value_name: "SECONDS",
                    help: "Maximum wait for each post page to hydrate. Defaults to 30.",
                    required: false,
                    kind: ArgKind::Int,
                },
            ],
            slow: SlowWhen::Always,
            run: run_get_posts,
        },
        SiteCommand {
            name: "page_state",
            tool_name: "page_state",
            about: "Open or reuse LinkedIn and print page state as JSON.",
            args: &[CommandArg {
                key: "wait_seconds",
                long: Some("wait-seconds"),
                value_name: "SECONDS",
                help: "Maximum wait for a LinkedIn page. Defaults to 30.",
                required: false,
                kind: ArgKind::Int,
            }],
            slow: SlowWhen::Always,
            run: run_page_state,
        },
    ],
};

fn run_search(
    page: Arc<PageSession>,
    args: Value,
    debug_snapshot: bool,
    progress: Option<ToolProgressSender>,
) -> BoxFuture<Value> {
    run_named(page, args, debug_snapshot, progress, "search", "search")
}

fn run_profile(
    page: Arc<PageSession>,
    args: Value,
    debug_snapshot: bool,
    progress: Option<ToolProgressSender>,
) -> BoxFuture<Value> {
    run_named(page, args, debug_snapshot, progress, "profile", "profile")
}

fn run_history(
    page: Arc<PageSession>,
    args: Value,
    debug_snapshot: bool,
    progress: Option<ToolProgressSender>,
) -> BoxFuture<Value> {
    run_named(page, args, debug_snapshot, progress, "history", "history")
}

fn run_company(
    page: Arc<PageSession>,
    args: Value,
    debug_snapshot: bool,
    progress: Option<ToolProgressSender>,
) -> BoxFuture<Value> {
    run_named(page, args, debug_snapshot, progress, "company", "company")
}

fn run_company_people(
    page: Arc<PageSession>,
    args: Value,
    debug_snapshot: bool,
    progress: Option<ToolProgressSender>,
) -> BoxFuture<Value> {
    run_named(
        page,
        args,
        debug_snapshot,
        progress,
        "company-people",
        "company_people",
    )
}

fn run_related_people(
    page: Arc<PageSession>,
    args: Value,
    debug_snapshot: bool,
    progress: Option<ToolProgressSender>,
) -> BoxFuture<Value> {
    run_named(
        page,
        args,
        debug_snapshot,
        progress,
        "related-people",
        "related_people",
    )
}

fn run_get_posts(
    page: Arc<PageSession>,
    args: Value,
    debug_snapshot: bool,
    progress: Option<ToolProgressSender>,
) -> BoxFuture<Value> {
    run_named(
        page,
        args,
        debug_snapshot,
        progress,
        "get-posts",
        "get_posts",
    )
}

fn run_page_state(
    page: Arc<PageSession>,
    args: Value,
    debug_snapshot: bool,
    progress: Option<ToolProgressSender>,
) -> BoxFuture<Value> {
    run_named(
        page,
        args,
        debug_snapshot,
        progress,
        "page_state",
        "page_state",
    )
}

fn run_named(
    page: Arc<PageSession>,
    args: Value,
    debug_snapshot: bool,
    progress: Option<ToolProgressSender>,
    command_name: &'static str,
    tool_name: &'static str,
) -> BoxFuture<Value> {
    run_skill_command(
        page.clone(),
        args,
        debug_snapshot,
        progress,
        ToolCommand {
            site_id: SITE_ID,
            command_name,
            tool_name,
            before: None,
            after: None,
            include_run_metadata: command_name == "get-posts",
        },
        linkedin_tools(page),
    )
}

struct SearchTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }

    fn description(&self) -> &str {
        "Search LinkedIn people, companies, or content matching `query`."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "maxLength": 512 },
                "result_type": { "type": "string" },
                "num": { "type": "integer", "default": 10, "minimum": 1, "maximum": 100 },
                "wait_seconds": { "type": "number", "default": 30, "minimum": 1, "maximum": 330 }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let query = required_string(&input, "query")?;
        if query.chars().count() > 512 {
            anyhow::bail!("query must contain at most 512 characters");
        }
        let result_type =
            normalize_result_type(get_str(&input, "result_type").unwrap_or("people"))?;
        let num = get_i64(&input, "num", DEFAULT_RESULT_COUNT).clamp(1, MAX_TOOL_ITEMS);
        let wait_seconds =
            get_f64(&input, "wait_seconds", DEFAULT_WAIT_SECONDS).clamp(1.0, MAX_TOOL_WAIT_SECONDS);
        let target = format!(
            "https://www.linkedin.com/search/results/{result_type}/?keywords={}",
            percent_encode_query(&query)
        );
        navigate_https(&self.page, &target).await?;
        let state_args = json!({ "query": query, "result_type": result_type });
        let state = wait_for_browser_tool(
            &self.page,
            SITE_ID,
            "searchState",
            Some(&state_args),
            wait_seconds,
        )
        .await?;
        if let Some(reason) = gate_reason(&state) {
            return Ok(json_result(&failure_payload(
                reason,
                json!({
                    "query": query,
                    "result_type": result_type,
                    "state": state,
                    "count": 0,
                    "results": [],
                }),
            )));
        }
        if !state.get("ok").and_then(Value::as_bool).unwrap_or(false)
            && !state.get("empty").and_then(Value::as_bool).unwrap_or(false)
        {
            return Ok(json_result(&failure_payload(
                state
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("search_results_unavailable"),
                json!({
                    "query": query,
                    "result_type": result_type,
                    "state": state,
                    "count": 0,
                    "results": [],
                }),
            )));
        }
        let results = if state.get("empty").and_then(Value::as_bool).unwrap_or(false) {
            Value::Array(Vec::new())
        } else {
            invoke_browser_tool(
                &self.page,
                ctx,
                SITE_ID,
                "searchResults",
                Some(&json!({ "limit": num, "result_type": result_type })),
                true,
            )
            .await?
        };
        Ok(json_result(&json!({
            "ok": true,
            "query": query,
            "result_type": result_type,
            "url": current_url(&self.page).await.unwrap_or_default(),
            "count": results.as_array().map(Vec::len).unwrap_or(0),
            "results": results,
            "state": state,
        })))
    }
}

struct ProfileTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for ProfileTool {
    fn name(&self) -> &str {
        "profile"
    }

    fn description(&self) -> &str {
        "Read a LinkedIn profile landing page by id or URL."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "profile": { "type": "string" },
                "wait_seconds": { "type": "number", "default": 30, "minimum": 1, "maximum": 330 }
            },
            "required": ["profile"]
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let locator = required_string(&input, "profile")?;
        let url = linkedin_profile_url(&locator)?;
        let wait_seconds =
            get_f64(&input, "wait_seconds", DEFAULT_WAIT_SECONDS).clamp(1.0, MAX_TOOL_WAIT_SECONDS);
        Ok(json_result(
            &read_object_page(&self.page, &url, "profileDetail", wait_seconds).await?,
        ))
    }
}

struct HistoryTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for HistoryTool {
    fn name(&self) -> &str {
        "history"
    }

    fn description(&self) -> &str {
        "Read complete visible work or education entries from a LinkedIn profile details route."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "profile": { "type": "string" },
                "section": { "type": "string" },
                "wait_seconds": { "type": "number", "default": 30, "minimum": 1, "maximum": 330 }
            },
            "required": ["profile"]
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let locator = required_string(&input, "profile")?;
        let section =
            normalize_history_section(get_str(&input, "section").unwrap_or("experience"))?;
        let url = linkedin_history_url(&locator, section)?;
        let wait_seconds =
            get_f64(&input, "wait_seconds", DEFAULT_WAIT_SECONDS).clamp(1.0, MAX_TOOL_WAIT_SECONDS);
        Ok(json_result(
            &read_object_page(&self.page, &url, "profileHistory", wait_seconds).await?,
        ))
    }
}

struct CompanyTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for CompanyTool {
    fn name(&self) -> &str {
        "company"
    }

    fn description(&self) -> &str {
        "Read a LinkedIn company or showcase landing page by id or URL."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "company": { "type": "string" },
                "wait_seconds": { "type": "number", "default": 30, "minimum": 1, "maximum": 330 }
            },
            "required": ["company"]
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let locator = required_string(&input, "company")?;
        let url = linkedin_company_url(&locator, false)?;
        let wait_seconds =
            get_f64(&input, "wait_seconds", DEFAULT_WAIT_SECONDS).clamp(1.0, MAX_TOOL_WAIT_SECONDS);
        Ok(json_result(
            &read_object_page(&self.page, &url, "companyDetail", wait_seconds).await?,
        ))
    }
}

struct CompanyPeopleTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for CompanyPeopleTool {
    fn name(&self) -> &str {
        "company_people"
    }

    fn description(&self) -> &str {
        "Read visible People you may know suggestions on a LinkedIn company people page."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "company": { "type": "string" },
                "num": { "type": "integer", "default": 10, "minimum": 1, "maximum": 100 },
                "wait_seconds": { "type": "number", "default": 30, "minimum": 1, "maximum": 330 }
            },
            "required": ["company"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let locator = required_string(&input, "company")?;
        let url = linkedin_company_url(&locator, true)?;
        let num = get_i64(&input, "num", DEFAULT_RESULT_COUNT).clamp(1, MAX_TOOL_ITEMS);
        let wait_seconds =
            get_f64(&input, "wait_seconds", DEFAULT_WAIT_SECONDS).clamp(1.0, MAX_TOOL_WAIT_SECONDS);
        navigate_https(&self.page, &url).await?;
        let state = wait_for_browser_tool(
            &self.page,
            SITE_ID,
            "companyPeople",
            Some(&json!({ "limit": num })),
            wait_seconds,
        )
        .await?;
        if let Some(reason) = gate_reason(&state) {
            return Ok(json_result(&failure_payload(
                reason,
                json!({ "company": locator, "url": url, "state": state }),
            )));
        }
        let people = invoke_browser_tool(
            &self.page,
            ctx,
            SITE_ID,
            "companyPeople",
            Some(&json!({ "limit": num })),
            false,
        )
        .await?;
        Ok(json_result(&people))
    }
}

struct RelatedPeopleTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for RelatedPeopleTool {
    fn name(&self) -> &str {
        "related_people"
    }

    fn description(&self) -> &str {
        "Read explicitly labelled related-people sections on a LinkedIn profile or company page."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target": { "type": "string" },
                "num": { "type": "integer", "default": 10, "minimum": 1, "maximum": 100 },
                "wait_seconds": { "type": "number", "default": 30, "minimum": 1, "maximum": 330 }
            },
            "required": ["target"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let locator = required_string(&input, "target")?;
        let url = linkedin_related_url(&locator)?;
        let num = get_i64(&input, "num", DEFAULT_RESULT_COUNT).clamp(1, MAX_TOOL_ITEMS);
        let wait_seconds =
            get_f64(&input, "wait_seconds", DEFAULT_WAIT_SECONDS).clamp(1.0, MAX_TOOL_WAIT_SECONDS);
        navigate_https(&self.page, &url).await?;
        let _ = wait_for_browser_tool(
            &self.page,
            SITE_ID,
            "relatedPeople",
            Some(&json!({ "limit": num })),
            wait_seconds,
        )
        .await?;
        let people = invoke_browser_tool(
            &self.page,
            ctx,
            SITE_ID,
            "relatedPeople",
            Some(&json!({ "limit": num })),
            false,
        )
        .await?;
        if let Some(reason) = gate_reason(&people) {
            return Ok(json_result(&failure_payload(
                reason,
                json!({ "target": locator, "url": url, "state": people }),
            )));
        }
        Ok(json_result(&people))
    }
}

struct GetPostsTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for GetPostsTool {
    fn name(&self) -> &str {
        "get_posts"
    }

    fn description(&self) -> &str {
        "Read one or more LinkedIn posts by URL, including comments."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "posts": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "num_comments": { "type": "integer", "default": 8, "minimum": 0, "maximum": 100 },
                "wait_seconds": { "type": "number", "default": 30, "minimum": 1, "maximum": 330 }
            },
            "required": ["posts"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let posts = input
            .get("posts")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("missing required argument: posts"))?;
        if posts.is_empty() {
            anyhow::bail!("at least one --post is required");
        }
        let num_comments =
            get_i64(&input, "num_comments", DEFAULT_COMMENT_COUNT).clamp(0, MAX_TOOL_ITEMS);
        let wait_seconds =
            get_f64(&input, "wait_seconds", DEFAULT_WAIT_SECONDS).clamp(1.0, MAX_TOOL_WAIT_SECONDS);
        let mut items = Vec::new();
        for post in posts {
            let locator = post
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("each --post must be a non-empty URL"))?;
            items.push(
                read_linkedin_post(&self.page, ctx, locator, num_comments, wait_seconds).await?,
            );
        }
        Ok(json_result(&json!({
            "ok": items.iter().all(|item| item.get("ok").and_then(Value::as_bool) == Some(true)),
            "count": items.len(),
            "posts": items,
        })))
    }
}

struct PageStateTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for PageStateTool {
    fn name(&self) -> &str {
        "page_state"
    }

    fn description(&self) -> &str {
        "Open or reuse LinkedIn and return the current route, login gate, and hydration state."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "wait_seconds": { "type": "number", "default": 30, "minimum": 1, "maximum": 330 }
            }
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let wait_seconds =
            get_f64(&input, "wait_seconds", DEFAULT_WAIT_SECONDS).clamp(1.0, MAX_TOOL_WAIT_SECONDS);
        ensure_site_page(&self.page, HOST_ROOT, HOME_URL).await?;
        let _ = wait_for_browser_tool(&self.page, SITE_ID, "pageState", None, wait_seconds).await?;
        let state = invoke_browser_tool(&self.page, ctx, SITE_ID, "pageState", None, false).await?;
        Ok(json_result(&state))
    }
}

async fn read_object_page(
    page: &PageSession,
    url: &str,
    tool_name: &str,
    wait_seconds: f64,
) -> anyhow::Result<Value> {
    navigate_https(page, url).await?;
    let state = wait_for_browser_tool(page, SITE_ID, tool_name, None, wait_seconds).await?;
    if let Some(reason) = gate_reason(&state) {
        return Ok(failure_payload(
            reason,
            json!({ "url": url, "state": state }),
        ));
    }
    if state.get("ok").and_then(Value::as_bool) != Some(true) {
        return Ok(failure_payload(
            state
                .get("status")
                .and_then(Value::as_str)
                .or_else(|| state.get("error").and_then(Value::as_str))
                .unwrap_or("page_unavailable"),
            json!({ "url": url, "state": state }),
        ));
    }
    Ok(state)
}

async fn read_linkedin_post(
    page: &PageSession,
    ctx: &ToolContext,
    locator: &str,
    num_comments: i64,
    wait_seconds: f64,
) -> anyhow::Result<Value> {
    let url = linkedin_post_url(locator)?;
    navigate_https(page, &url).await?;
    let detail = wait_for_browser_tool(page, SITE_ID, "postDetail", None, wait_seconds).await?;
    if let Some(reason) = gate_reason(&detail) {
        return Ok(failure_payload(
            reason,
            json!({ "input": locator, "url": url, "entity": detail }),
        ));
    }
    if detail.get("ok").and_then(Value::as_bool) != Some(true) {
        return Ok(failure_payload(
            detail
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("post_unavailable"),
            json!({ "input": locator, "url": url, "entity": detail }),
        ));
    }
    let entity = invoke_browser_tool(page, ctx, SITE_ID, "postDetail", None, false).await?;
    let comments = if num_comments > 0 {
        invoke_browser_tool(
            page,
            ctx,
            SITE_ID,
            "comments",
            Some(&json!({ "limit": num_comments })),
            true,
        )
        .await?
    } else {
        Value::Array(Vec::new())
    };
    Ok(json!({
        "ok": true,
        "input": locator,
        "url": current_url(page).await.unwrap_or(url),
        "entity": entity,
        "comments": comments,
    }))
}

fn normalize_result_type(value: &str) -> anyhow::Result<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "people" => Ok("people"),
        "content" => Ok("content"),
        "companies" | "company" => Ok("companies"),
        "all" => Ok("all"),
        other => anyhow::bail!("unsupported LinkedIn search type: {other}"),
    }
}

fn normalize_history_section(value: &str) -> anyhow::Result<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "experience" | "work" => Ok("experience"),
        "education" => Ok("education"),
        other => anyhow::bail!("unsupported LinkedIn history section: {other}"),
    }
}

fn linkedin_https_url(locator: &str) -> Option<String> {
    reqwest::Url::parse(locator.trim())
        .ok()
        .filter(|url| url.scheme() == "https")
        .map(|url| url.to_string())
}

fn linkedin_slug(locator: &str) -> anyhow::Result<String> {
    let trimmed = locator.trim().trim_matches('/');
    if trimmed.is_empty()
        || trimmed.contains('/')
        || !trimmed.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '%')
        })
    {
        anyhow::bail!("invalid LinkedIn id: {locator}");
    }
    Ok(trimmed.to_string())
}

fn linkedin_profile_url(locator: &str) -> anyhow::Result<String> {
    if let Some(url) = linkedin_https_url(locator) {
        return Ok(url);
    }
    Ok(format!(
        "https://www.linkedin.com/in/{}/",
        linkedin_slug(locator)?
    ))
}

fn linkedin_history_url(locator: &str, section: &str) -> anyhow::Result<String> {
    if let Some(url) = linkedin_https_url(locator) {
        if url.contains("/details/") {
            return Ok(url);
        }
        let parsed = reqwest::Url::parse(&url)?;
        let id = parsed
            .path_segments()
            .and_then(|mut parts| {
                let mut found = None;
                while let Some(part) = parts.next() {
                    if part == "in" {
                        found = parts.next().map(ToOwned::to_owned);
                        break;
                    }
                }
                found
            })
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("LinkedIn history URL must include /in/<id>"))?;
        return Ok(format!(
            "https://www.linkedin.com/in/{id}/details/{section}/"
        ));
    }
    Ok(format!(
        "https://www.linkedin.com/in/{}/details/{section}/",
        linkedin_slug(locator)?
    ))
}

fn linkedin_company_url(locator: &str, people: bool) -> anyhow::Result<String> {
    if let Some(url) = linkedin_https_url(locator) {
        if !people {
            return Ok(url);
        }
        if url.contains("/people") {
            return Ok(url);
        }
        let trimmed = url.trim_end_matches('/');
        return Ok(format!("{trimmed}/people/"));
    }
    let slug = linkedin_slug(locator)?;
    if people {
        Ok(format!("https://www.linkedin.com/company/{slug}/people/"))
    } else {
        Ok(format!("https://www.linkedin.com/company/{slug}/"))
    }
}

fn linkedin_related_url(locator: &str) -> anyhow::Result<String> {
    if let Some(url) = linkedin_https_url(locator) {
        return Ok(url);
    }
    Ok(format!(
        "https://www.linkedin.com/in/{}/",
        linkedin_slug(locator)?
    ))
}

fn linkedin_post_url(locator: &str) -> anyhow::Result<String> {
    linkedin_https_url(locator)
        .filter(|url| url.contains("/posts/") || url.contains("/feed/update/"))
        .ok_or_else(|| {
            anyhow::anyhow!("LinkedIn get-posts requires a /posts/ or /feed/update/ URL")
        })
}
