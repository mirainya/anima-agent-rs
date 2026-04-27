use anima_runtime::pipeline::*;
use serde_json::{json, Value};

// ── Pipeline basic flow ─────────────────────────────────────────────

#[test]
fn pipeline_process_single_item_through_transform_and_sink() {
    let pipeline = Pipeline::new().add_transform(Box::new(MapTransform::new(|v: Value| {
        json!(v.as_i64().unwrap_or(0) * 2)
    })));

    let result = pipeline.process(json!(5)).unwrap();
    assert_eq!(result, Some(json!(10)));
}

#[test]
fn pipeline_filter_drops_items() {
    let pipeline = Pipeline::new()
        .add_filter(Box::new(PredicateFilter::new(|v: &Value| {
            v.as_i64().unwrap_or(0) > 3
        })))
        .add_transform(Box::new(MapTransform::new(|v: Value| {
            json!(v.as_i64().unwrap_or(0) * 10)
        })));

    assert_eq!(pipeline.process(json!(2)).unwrap(), None); // filtered
    assert_eq!(pipeline.process(json!(5)).unwrap(), Some(json!(50))); // passes
}

#[test]
fn pipeline_multiple_transforms_chain() {
    let pipeline = Pipeline::new()
        .add_transform(Box::new(MapTransform::new(|v: Value| {
            json!(v.as_i64().unwrap_or(0) + 1)
        })))
        .add_transform(Box::new(MapTransform::new(|v: Value| {
            json!(v.as_i64().unwrap_or(0) * 3)
        })));

    // (5 + 1) * 3 = 18
    assert_eq!(pipeline.process(json!(5)).unwrap(), Some(json!(18)));
}

#[test]
fn pipeline_multiple_filters_all_must_pass() {
    let pipeline = Pipeline::new()
        .add_filter(Box::new(PredicateFilter::new(|v: &Value| {
            v.as_i64().unwrap_or(0) > 0
        })))
        .add_filter(Box::new(PredicateFilter::new(|v: &Value| {
            v.as_i64().unwrap_or(0) < 100
        })));

    assert_eq!(pipeline.process(json!(-1)).unwrap(), None);
    assert_eq!(pipeline.process(json!(50)).unwrap(), Some(json!(50)));
    assert_eq!(pipeline.process(json!(200)).unwrap(), None);
}

// ── Aggregate ───────────────────────────────────────────────────────

#[test]
fn pipeline_sum_aggregate() {
    let pipeline = Pipeline::new().add_aggregate(Box::new(SumAggregate::new()));

    // Aggregate absorbs items — returns None per-item
    assert_eq!(pipeline.process(json!(10)).unwrap(), None);
    assert_eq!(pipeline.process(json!(20)).unwrap(), None);
    assert_eq!(pipeline.process(json!(30)).unwrap(), None);

    let result = pipeline.aggregation_result().unwrap();
    assert_eq!(result["sum"], json!(60.0));
    assert_eq!(result["count"], json!(3));
}

#[test]
fn pipeline_collect_aggregate() {
    let pipeline = Pipeline::new().add_aggregate(Box::new(CollectAggregate::new()));

    pipeline.process(json!("a")).unwrap();
    pipeline.process(json!("b")).unwrap();
    pipeline.process(json!("c")).unwrap();

    let result = pipeline.aggregation_result().unwrap();
    assert_eq!(result, json!(["a", "b", "c"]));
}

// ── Batch transform ─────────────────────────────────────────────────

#[test]
fn batch_transform_groups_items() {
    let pipeline = Pipeline::new().add_transform(Box::new(BatchTransform::new(3)));

    assert_eq!(pipeline.process(json!(1)).unwrap(), None);
    assert_eq!(pipeline.process(json!(2)).unwrap(), None);
    // Third item triggers batch
    let result = pipeline.process(json!(3)).unwrap();
    assert_eq!(result, Some(json!([1, 2, 3])));
}

// ── Stats ───────────────────────────────────────────────────────────

#[test]
fn pipeline_tracks_stats() {
    let pipeline = Pipeline::new().add_filter(Box::new(PredicateFilter::new(|v: &Value| {
        v.as_i64().unwrap_or(0) > 0
    })));

    pipeline.process(json!(1)).unwrap();
    pipeline.process(json!(-1)).unwrap();
    pipeline.process(json!(2)).unwrap();

    let stats = pipeline.stats();
    assert_eq!(stats.items_processed, 3);
    assert_eq!(stats.items_filtered, 1);
    assert_eq!(stats.errors, 0);
}

// ── Process batch ───────────────────────────────────────────────────

#[test]
fn pipeline_process_batch() {
    let pipeline = Pipeline::new().add_transform(Box::new(MapTransform::new(|v: Value| {
        json!(v.as_i64().unwrap_or(0) + 100)
    })));

    let results = pipeline.process_batch(vec![json!(1), json!(2), json!(3)]);
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].as_ref().unwrap(), &Some(json!(101)));
    assert_eq!(results[1].as_ref().unwrap(), &Some(json!(102)));
    assert_eq!(results[2].as_ref().unwrap(), &Some(json!(103)));
}

// ── Channel source + sink ───────────────────────────────────────────

#[test]
fn channel_source_and_sink_integration() {
    let (src_tx, src_rx) = crossbeam_channel::bounded(10);
    let (sink_tx, _sink_rx) = crossbeam_channel::bounded(10);

    let source = ChannelSource::new(src_rx);
    let pipeline = Pipeline::new()
        .add_source(Box::new(source))
        .add_transform(Box::new(MapTransform::new(|v: Value| {
            json!(v.as_i64().unwrap_or(0) * 2)
        })))
        .add_sink(Box::new(ChannelSink::new(sink_tx)));

    pipeline.start().unwrap();

    src_tx.send(json!(5)).unwrap();
    src_tx.send(json!(10)).unwrap();

    pipeline.stop().unwrap();
    assert_eq!(pipeline.state(), PipelineState::Stopped);
}

// ── Collection source ───────────────────────────────────────────────

#[test]
fn collection_source_emits_items() {
    let source = CollectionSource::new(vec![json!(1), json!(2), json!(3)]);
    assert_eq!(source.remaining(), 3);
    assert_eq!(source.next_item(), Some(json!(1)));
    assert_eq!(source.next_item(), Some(json!(2)));
    assert_eq!(source.remaining(), 1);
    assert_eq!(source.next_item(), Some(json!(3)));
    assert_eq!(source.next_item(), None);
}

// ── Null sink ───────────────────────────────────────────────────────

#[test]
fn null_sink_discards_everything() {
    let sink = NullSink::new();
    let ctx = PipelineContext::new("test");
    sink.write(json!(1), &ctx).unwrap();
    sink.write(json!(2), &ctx).unwrap();
    assert_eq!(sink.count(), 2);
}
