use serde::{Deserialize, Serialize};

/// Input for the Bash tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BashInput {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_in_background: Option<bool>,
}

/// Input for the Read tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReadInput {
    pub file_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<String>,
}

/// Input for the Write tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileWriteInput {
    pub file_path: String,
    pub content: String,
}

/// Input for the Edit tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEditInput {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replace_all: Option<bool>,
}

/// Input for the Glob tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobInput {
    pub pattern: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Input for the Grep tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrepInput {
    pub pattern: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub file_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_mode: Option<GrepOutputMode>,
    #[serde(rename = "-i", skip_serializing_if = "Option::is_none")]
    pub case_insensitive: Option<bool>,
    #[serde(rename = "-n", skip_serializing_if = "Option::is_none")]
    pub line_numbers: Option<bool>,
    #[serde(rename = "-B", skip_serializing_if = "Option::is_none")]
    pub before_context: Option<u32>,
    #[serde(rename = "-A", skip_serializing_if = "Option::is_none")]
    pub after_context: Option<u32>,
    #[serde(rename = "-C", skip_serializing_if = "Option::is_none")]
    pub context_alias: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiline: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GrepOutputMode {
    Content,
    FilesWithMatches,
    Count,
}

/// Input for the WebFetch tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebFetchInput {
    pub url: String,
    pub prompt: String,
}

/// Input for the WebSearch tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchInput {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_domains: Option<Vec<String>>,
}

/// Input for the AskUserQuestion tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskUserQuestionInput {
    pub questions: Vec<Question>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub question: String,
    pub header: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

/// Input for the NotebookEdit tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookEditInput {
    pub notebook_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_id: Option<String>,
    pub new_source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_type: Option<CellType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit_mode: Option<EditMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CellType {
    Code,
    Markdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EditMode {
    Replace,
    Insert,
    Delete,
}

/// Input for the TodoWrite tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoWriteInput {
    pub todos: Vec<TodoItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
    pub active_form: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}
