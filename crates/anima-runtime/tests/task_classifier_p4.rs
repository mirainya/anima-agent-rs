use anima_runtime::task_classifier::*;

// ── Code Generation ─────────────────────────────────────────────────

#[test]
fn classifies_code_generation_english() {
    let r = classify_task("Write a function to sort a list");
    assert_eq!(r.task_type, TaskType::CodeGeneration);
    assert!(r.confidence > 0.1);
    assert!(r.detected_keywords.contains(&"write".to_string()));
}

#[test]
fn classifies_code_generation_chinese() {
    let r = classify_task("编写一个排序函数");
    assert_eq!(r.task_type, TaskType::CodeGeneration);
}

#[test]
fn classifies_implement_pattern() {
    let r = classify_task("Implement a cache layer");
    assert_eq!(r.task_type, TaskType::CodeGeneration);
}

// ── Code Review ─────────────────────────────────────────────────────

#[test]
fn classifies_code_review() {
    let r = classify_task("Review this code for bugs");
    assert_eq!(r.task_type, TaskType::CodeReview);
}

#[test]
fn classifies_code_review_chinese() {
    let r = classify_task("代码审查这段代码");
    assert_eq!(r.task_type, TaskType::CodeReview);
}

// ── Code Debug ──────────────────────────────────────────────────────

#[test]
fn classifies_debug() {
    let r = classify_task("Fix this error in the login module");
    assert_eq!(r.task_type, TaskType::CodeDebug);
}

#[test]
fn classifies_debug_bug() {
    let r = classify_task("There is a bug in the parser");
    assert_eq!(r.task_type, TaskType::CodeDebug);
}

// ── Code Refactor ───────────────────────────────────────────────────

#[test]
fn classifies_refactor() {
    let r = classify_task("Refactor this code to be cleaner");
    assert_eq!(r.task_type, TaskType::CodeRefactor);
}

#[test]
fn classifies_optimize() {
    let r = classify_task("Optimize this code for performance");
    assert_eq!(r.task_type, TaskType::CodeRefactor);
}

// ── Web Search ──────────────────────────────────────────────────────

#[test]
fn classifies_web_search() {
    let r = classify_task("Search for information about Rust async");
    assert_eq!(r.task_type, TaskType::WebSearch);
}

// ── Documentation ───────────────────────────────────────────────────

#[test]
fn classifies_documentation() {
    let r = classify_task("Write documentation for the API");
    // "write" matches code-gen, "documentation" matches docs
    // Pattern "write documentation" should match docs
    assert!(
        r.task_type == TaskType::Documentation || r.task_type == TaskType::CodeGeneration,
        "Expected Documentation or CodeGeneration, got {:?}",
        r.task_type
    );
}

#[test]
fn classifies_explain() {
    let r = classify_task("Explain how this algorithm works");
    assert_eq!(r.task_type, TaskType::Documentation);
}

// ── Test Generation ─────────────────────────────────────────────────

#[test]
fn classifies_test_generation() {
    let r = classify_task("Write tests for the user service");
    // "write" matches code-gen, "tests" matches test-gen
    assert!(
        r.task_type == TaskType::TestGeneration || r.task_type == TaskType::CodeGeneration,
        "Expected TestGeneration or CodeGeneration, got {:?}",
        r.task_type
    );
}

// ── General Chat ────────────────────────────────────────────────────

#[test]
fn classifies_greeting() {
    let r = classify_task("Hello, how are you?");
    assert_eq!(r.task_type, TaskType::GeneralChat);
}

#[test]
fn classifies_chinese_greeting() {
    let r = classify_task("你好");
    assert_eq!(r.task_type, TaskType::GeneralChat);
}

// ── Result structure ────────────────────────────────────────────────

#[test]
fn result_has_all_scores() {
    let r = classify_task("Write a function");
    assert_eq!(r.scores.len(), 10); // all 10 task types
    assert!(r.scores.values().all(|&s| s > 0)); // all have base priority score
}

#[test]
fn result_tracks_message_length() {
    let msg = "Hello world";
    let r = classify_task(msg);
    assert_eq!(r.message_length, msg.len());
}

#[test]
fn result_detects_code_blocks() {
    let r = classify_task("Fix this: ```rust\nfn main() {}\n```");
    assert!(r.has_code_block);

    let r2 = classify_task("Just a normal message");
    assert!(!r2.has_code_block);
}

// ── Specialist mapping ──────────────────────────────────────────────

#[test]
fn specialist_mapping() {
    assert_eq!(TaskType::CodeGeneration.specialist(), "code-specialist");
    assert_eq!(TaskType::CodeReview.specialist(), "code-specialist");
    assert_eq!(TaskType::CodeDebug.specialist(), "code-specialist");
    assert_eq!(TaskType::WebSearch.specialist(), "search-specialist");
    assert_eq!(TaskType::Documentation.specialist(), "doc-specialist");
    assert_eq!(TaskType::DataAnalysis.specialist(), "analysis-specialist");
    assert_eq!(TaskType::GeneralChat.specialist(), "chat-specialist");
    assert_eq!(TaskType::ApiCall.specialist(), "general-specialist");
}

// ── Description ─────────────────────────────────────────────────────

#[test]
fn task_type_descriptions() {
    assert!(TaskType::CodeGeneration.description().contains("代码生成"));
    assert!(TaskType::GeneralChat.description().contains("普通对话"));
}

#[test]
fn task_type_as_str() {
    assert_eq!(TaskType::CodeGeneration.as_str(), "code-generation");
    assert_eq!(TaskType::GeneralChat.as_str(), "general-chat");
}

// ── Language Detection ──────────────────────────────────────────────

#[test]
fn detects_rust() {
    assert_eq!(detect_language("fn main() { let mut x = 5; }"), Some("rust"));
}

#[test]
fn detects_python() {
    assert_eq!(detect_language("def hello():\n    pass"), Some("python"));
}

#[test]
fn detects_no_language() {
    assert_eq!(detect_language("Hello world"), None);
}

// ── Code Block Extraction ────────────────────────────────────────────

#[test]
fn extract_code_blocks_fenced_and_inline() {
    let msg = "Use `foo()` or:\n```rust\nfn bar() {}\n```";
    let blocks = extract_code_blocks(msg);
    assert!(blocks.has_code);
    assert_eq!(blocks.fenced.len(), 1);
    assert!(blocks.fenced[0].contains("fn bar()"));
    assert_eq!(blocks.inline.len(), 1);
    assert_eq!(blocks.inline[0], "foo()");
}

#[test]
fn extract_code_blocks_no_code() {
    let blocks = extract_code_blocks("Just a plain message with no code.");
    assert!(!blocks.has_code);
    assert!(blocks.fenced.is_empty());
    assert!(blocks.inline.is_empty());
}

// ── Task Context ─────────────────────────────────────────────────────

#[test]
fn extract_task_context_combines_results() {
    let msg = "URGENT: fix this bug in `main()` immediately";
    let ctx = extract_task_context(msg);
    assert_eq!(ctx.classification.task_type, TaskType::CodeDebug);
    assert!(ctx.is_urgent);
    assert!(ctx.code_blocks.has_code);
    assert_eq!(ctx.code_blocks.inline[0], "main()");
}

// ── Multiple Task Detection ──────────────────────────────────────────

#[test]
fn detect_multiple_tasks_splits_then() {
    let results = detect_multiple_tasks("search for docs then write a function");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].task_type, TaskType::WebSearch);
    assert_eq!(results[1].task_type, TaskType::CodeGeneration);
}

#[test]
fn detect_multiple_tasks_single_returns_vec_of_one() {
    let results = detect_multiple_tasks("Write a sorting function");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].task_type, TaskType::CodeGeneration);
}
