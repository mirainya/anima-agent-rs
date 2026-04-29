use anima_runtime::bus::{make_inbound, MakeInbound};
use anima_runtime::cache::LruCache;
use anima_runtime::classifier::ai::*;
use anima_runtime::classifier::rule::ClassificationKind;
use serde_json::json;
use std::sync::Arc;

// ── Quick pattern classification ────────────────────────────────────

#[test]
fn quick_classify_greetings() {
    let r = quick_classify("hi").unwrap();
    assert_eq!(r.classification, AiClassificationType::SimpleChat);
    assert_eq!(r.confidence, 1.0);
    assert!(r.quick);

    let r = quick_classify("你好").unwrap();
    assert_eq!(r.classification, AiClassificationType::SimpleChat);

    let r = quick_classify("Hello").unwrap();
    assert_eq!(r.classification, AiClassificationType::SimpleChat);
}

#[test]
fn quick_classify_system_commands() {
    let r = quick_classify("exit").unwrap();
    assert_eq!(r.classification, AiClassificationType::SystemCommand);

    let r = quick_classify("退出").unwrap();
    assert_eq!(r.classification, AiClassificationType::SystemCommand);
}

#[test]
fn quick_classify_status_queries() {
    let r = quick_classify("status").unwrap();
    assert_eq!(r.classification, AiClassificationType::StatusQuery);

    let r = quick_classify("任务状态").unwrap();
    assert_eq!(r.classification, AiClassificationType::StatusQuery);
}

#[test]
fn quick_classify_complex_tasks() {
    let r = quick_classify("Write a function to sort arrays").unwrap();
    assert_eq!(r.classification, AiClassificationType::ComplexTask);

    let r = quick_classify("帮我写个函数").unwrap();
    assert_eq!(r.classification, AiClassificationType::ComplexTask);

    let r = quick_classify("fix the bug in login").unwrap();
    assert_eq!(r.classification, AiClassificationType::ComplexTask);

    let r = quick_classify("refactor the code").unwrap();
    assert_eq!(r.classification, AiClassificationType::ComplexTask);
}

#[test]
fn quick_classify_returns_none_for_ambiguous() {
    assert!(quick_classify("tell me about rust").is_none());
    assert!(quick_classify("what is a monad").is_none());
}

// ── AI response parsing ─────────────────────────────────────────────

#[test]
fn parse_valid_ai_response() {
    let response =
        r#"{"type": "complex-task", "confidence": 0.92, "reasoning": "code generation"}"#;
    let r = AiClassifier::parse_response(response);
    assert_eq!(r.classification, AiClassificationType::ComplexTask);
    assert!((r.confidence - 0.92).abs() < 0.01);
    assert_eq!(r.reasoning, "code generation");
    assert!(!r.quick);
}

#[test]
fn parse_ai_response_with_surrounding_text() {
    let response = r#"Here is my analysis: {"type": "simple-chat", "confidence": 0.85, "reasoning": "greeting"} That's my answer."#;
    let r = AiClassifier::parse_response(response);
    assert_eq!(r.classification, AiClassificationType::SimpleChat);
}

#[test]
fn parse_invalid_ai_response_falls_back() {
    let r = AiClassifier::parse_response("I don't know what to say");
    assert_eq!(r.classification, AiClassificationType::SimpleChat);
    assert_eq!(r.confidence, 0.5);
}

// ── Cache integration ───────────────────────────────────────────────

#[test]
fn classifier_caches_results() {
    let cache = Arc::new(LruCache::new(None, None));
    let classifier = AiClassifier::new(cache);

    let result = AiClassificationResult {
        classification: AiClassificationType::ComplexTask,
        confidence: 0.9,
        reasoning: "test".into(),
        quick: false,
    };
    classifier.cache_result("build a website", &result);

    let cached = classifier.get_cached("build a website").unwrap();
    assert_eq!(cached.classification, AiClassificationType::ComplexTask);
    assert!((cached.confidence - 0.9).abs() < 0.01);
}

#[test]
fn classifier_cache_disabled() {
    let cache = Arc::new(LruCache::new(None, None));
    let classifier = AiClassifier::new(cache).with_cache(false);

    let result = AiClassificationResult {
        classification: AiClassificationType::ComplexTask,
        confidence: 0.9,
        reasoning: "test".into(),
        quick: false,
    };
    classifier.cache_result("build a website", &result);
    assert!(classifier.get_cached("build a website").is_none());
}

// ── Smart classify ──────────────────────────────────────────────────

#[test]
fn smart_classify_prefers_quick_patterns() {
    let cache = Arc::new(LruCache::new(None, None));
    let classifier = AiClassifier::new(cache);

    let r = smart_classify(Some(&classifier), "hi", None);
    assert_eq!(r.classification, AiClassificationType::SimpleChat);
    assert!(r.quick);
}

#[test]
fn smart_classify_uses_ai_response_when_no_pattern() {
    let cache = Arc::new(LruCache::new(None, None));
    let classifier = AiClassifier::new(cache);

    let ai_response = r#"{"type": "complex-task", "confidence": 0.88, "reasoning": "needs code"}"#;
    let r = smart_classify(Some(&classifier), "tell me about rust", Some(ai_response));
    assert_eq!(r.classification, AiClassificationType::ComplexTask);
    assert!(!r.quick);
}

#[test]
fn smart_classify_fallback_without_classifier() {
    let r = smart_classify(None, "something ambiguous", None);
    assert_eq!(r.classification, AiClassificationType::SimpleChat);
    assert_eq!(r.confidence, 0.5);
}

// ── Bridge to rule-based classification ─────────────────────────────

#[test]
fn ai_to_rule_bridge_maps_correctly() {
    let msg = make_inbound(MakeInbound {
        channel: "test".into(),
        content: "hello".into(),
        ..Default::default()
    });

    let simple = AiClassificationResult {
        classification: AiClassificationType::SimpleChat,
        confidence: 0.9,
        reasoning: "chat".into(),
        quick: true,
    };
    assert_eq!(
        ai_to_rule_classification(&simple, &msg).kind,
        ClassificationKind::Single
    );

    let system = AiClassificationResult {
        classification: AiClassificationType::SystemCommand,
        confidence: 1.0,
        reasoning: "exit".into(),
        quick: true,
    };
    assert_eq!(
        ai_to_rule_classification(&system, &msg).kind,
        ClassificationKind::Direct
    );

    let complex = AiClassificationResult {
        classification: AiClassificationType::ComplexTask,
        confidence: 0.9,
        reasoning: "code".into(),
        quick: false,
    };
    assert_eq!(
        ai_to_rule_classification(&complex, &msg).kind,
        ClassificationKind::Single
    );
}

#[test]
fn ai_to_rule_bridge_complex_with_specialist() {
    let msg = make_inbound(MakeInbound {
        channel: "test".into(),
        content: "build api".into(),
        metadata: Some(json!({"specialist": "backend"})),
        ..Default::default()
    });

    let complex = AiClassificationResult {
        classification: AiClassificationType::ComplexTask,
        confidence: 0.9,
        reasoning: "code".into(),
        quick: false,
    };
    assert_eq!(
        ai_to_rule_classification(&complex, &msg).kind,
        ClassificationKind::SpecialistRoute
    );
}

#[test]
fn ai_to_rule_bridge_complex_with_parallel_maps_to_single() {
    let msg = make_inbound(MakeInbound {
        channel: "test".into(),
        content: "process data".into(),
        metadata: Some(json!({"parallel": true})),
        ..Default::default()
    });

    let complex = AiClassificationResult {
        classification: AiClassificationType::ComplexTask,
        confidence: 0.9,
        reasoning: "parallel work".into(),
        quick: false,
    };
    assert_eq!(
        ai_to_rule_classification(&complex, &msg).kind,
        ClassificationKind::Single
    );
}
