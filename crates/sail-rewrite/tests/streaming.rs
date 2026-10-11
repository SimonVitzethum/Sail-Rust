//! A corpus larger than the test cgroup is rewritten, spilled and read back.
#![forbid(unsafe_code)]

use sail_ooc::{ContentId, WarmLayer};
use sail_rewrite::{Def, DefKind, Pipeline, Shard, Stmt};
use sail_shard::SchedulerConfig;
use std::io::{self, Read};

#[test]
fn streaming_rewrites_and_spills_a_corpus_larger_than_ram() {
    let dir = std::env::temp_dir().join(format!("sail-rewrite-stream-{}", std::process::id()));
    let warm = WarmLayer::open(&dir).with_max_live_bytes(0);
    let shards = (0..512).map(|id| Shard::new(id, vec![Def::new(
        format!("f{id}"), DefKind::Function, vec![
            Stmt::Undefined { target: "R".into(), ty: "bits(64)".into() },
            Stmt::Expr("x".repeat(512 * 1024)),
        ],
    )]));
    let mut processed = 0;
    let mut total = 0;
    let result = Pipeline::lean_implemented().run_streaming(shards,
        SchedulerConfig::new(2, 8), |order, shard| {
            assert_eq!(order, processed);
            assert_eq!(shard.id, order);
            assert_eq!(shard.defs[0].body[0], Stmt::Assign {
                lhs: "R".into(), rhs: "undefined_bitvector(sizeof(64))".into(),
            });
            let Stmt::Expr(payload) = &shard.defs[0].body[1] else { panic!("payload lost") };
            // Include order in the ID so this creates an actual 256 MiB corpus.
            let id = ContentId::of_domain_parts("rewrite-test", &[&order.to_le_bytes(), payload.as_bytes()]);
            warm.store(&id, payload.as_bytes()).map_err(io::Error::other)?;
            let mut reader = warm.reader(&id).map_err(io::Error::other)?.unwrap();
            let mut first = [0; 1];
            reader.read_exact(&mut first)?;
            assert_eq!(first, [b'x']);
            total += 1 + io::copy(&mut reader, &mut io::sink())?;
            // Consumer has emitted this shard; remove it so disk usage also
            // stays bounded in repeated CI runs.
            std::fs::remove_file(warm.blob_path(&id))?;
            processed += 1;
            assert_eq!(warm.live_bytes(), 0);
            Ok::<_, io::Error>(())
        });
    let _ = std::fs::remove_dir_all(dir);
    result.unwrap();
    assert_eq!(processed, 512);
    assert_eq!(total, 256 * 1024 * 1024);
}
