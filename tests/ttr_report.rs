//! Time-to-Recovery (TTR) Benchmark Suite
//!
//! Measures recovery times for various cluster failure scenarios.
//! Produces a formatted report with min/median/max timings.

mod cluster_battle;

use cluster_battle::harness::TestCluster;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Timing helpers
// ---------------------------------------------------------------------------

struct TimingResult {
    scenario: String,
    description: String,
    samples: Vec<Duration>,
}

impl TimingResult {
    fn min(&self) -> Duration {
        *self.samples.iter().min().unwrap()
    }
    fn max(&self) -> Duration {
        *self.samples.iter().max().unwrap()
    }
    fn median(&self) -> Duration {
        let mut sorted = self.samples.clone();
        sorted.sort();
        sorted[sorted.len() / 2]
    }
    fn mean(&self) -> Duration {
        let total: Duration = self.samples.iter().sum();
        total / self.samples.len() as u32
    }
    fn p95(&self) -> Duration {
        let mut sorted = self.samples.clone();
        sorted.sort();
        let idx = ((sorted.len() as f64) * 0.95).ceil() as usize - 1;
        sorted[idx.min(sorted.len() - 1)]
    }
}

fn fmt_ms(d: Duration) -> String {
    format!("{:.1}ms", d.as_secs_f64() * 1000.0)
}

fn print_report(results: &[TimingResult]) {
    println!();
    println!("==================================================================================================");
    println!("                     SYNAPTICA CLUSTER -- TIME TO RECOVERY REPORT                                  ");
    println!("==================================================================================================");
    println!();
    println!(
        "  Config: heartbeat=100ms, election_timeout=300-600ms, in-process network (zero latency)"
    );
    println!("  Each scenario measured 3 times. All timings in milliseconds.");
    println!();
    println!("------------------------------------------+-------+---------+-------+-------+---------+--------");
    println!(" Scenario                                  |  Min  | Median  |  Mean |  Max  |   P95   |Samples");
    println!("------------------------------------------+-------+---------+-------+-------+---------+--------");

    for r in results {
        println!(
            " {:<41}|{:>6} |{:>8} |{:>6} |{:>6} |{:>8} |   {}   ",
            r.scenario,
            fmt_ms(r.min()),
            fmt_ms(r.median()),
            fmt_ms(r.mean()),
            fmt_ms(r.max()),
            fmt_ms(r.p95()),
            r.samples.len(),
        );
    }

    println!("------------------------------------------+-------+---------+-------+-------+---------+--------");
    println!();
    println!("  DEFINITIONS");
    println!("  ----------");
    for r in results {
        let desc = if r.description.len() > 88 {
            format!("{}...", &r.description[..85])
        } else {
            r.description.clone()
        };
        println!("  * {:<42}: {}", r.scenario, desc);
    }
    println!();
    println!("==================================================================================================");
    println!();
}

// ---------------------------------------------------------------------------
// Measurement helpers
// ---------------------------------------------------------------------------

/// Measure time until a new leader is elected (different from old_leader).
async fn measure_leader_election(cluster: &TestCluster, old_leader: u64) -> Duration {
    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(30) {
            panic!("timeout waiting for new leader election");
        }
        if let Some(new_leader) = cluster.get_leader() {
            if new_leader != old_leader {
                return start.elapsed();
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Measure time until a specific node has the expected node count.
async fn measure_replication_to(cluster: &TestCluster, node_id: u64, expected: usize) -> Duration {
    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(30) {
            panic!(
                "timeout waiting for node {} to have {} nodes (has {})",
                node_id,
                expected,
                cluster.count_nodes_on(node_id),
            );
        }
        if cluster.count_nodes_on(node_id) == expected {
            return start.elapsed();
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Measure time until all (non-blocked) nodes converge to the expected count.
async fn measure_convergence(cluster: &TestCluster, expected: usize) -> Duration {
    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(30) {
            panic!("timeout waiting for convergence to {}", expected);
        }
        let all = cluster
            .nodes
            .keys()
            .all(|id| cluster.count_nodes_on(*id) == expected);
        if all {
            return start.elapsed();
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Measure time for a write to commit through Raft.
async fn measure_write_latency(cluster: &TestCluster, query: &str) -> Duration {
    let start = Instant::now();
    let resp = cluster.write(query).await.unwrap();
    assert!(resp.success, "write failed: {:?}", resp.error);
    start.elapsed()
}

// ---------------------------------------------------------------------------
// Main benchmark
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ttr_recovery_report() {
    let mut results: Vec<TimingResult> = Vec::new();
    let runs = 3;

    // -----------------------------------------------------------------------
    // 1. Leader election time (3-node cold start)
    // -----------------------------------------------------------------------
    {
        let mut samples = Vec::new();
        for _ in 0..runs {
            let start = Instant::now();
            let cluster = TestCluster::new(3).await; // waits for leader
            samples.push(start.elapsed());
            cluster.shutdown().await;
        }
        results.push(TimingResult {
            scenario: "Cold start election (3-node)".into(),
            description: "Time from cluster creation to first leader elected".into(),
            samples,
        });
    }

    // -----------------------------------------------------------------------
    // 2. Leader election time (5-node cold start)
    // -----------------------------------------------------------------------
    {
        let mut samples = Vec::new();
        for _ in 0..runs {
            let start = Instant::now();
            let cluster = TestCluster::new(5).await;
            samples.push(start.elapsed());
            cluster.shutdown().await;
        }
        results.push(TimingResult {
            scenario: "Cold start election (5-node)".into(),
            description: "Time from 5-node cluster creation to first leader".into(),
            samples,
        });
    }

    // -----------------------------------------------------------------------
    // 3. Re-election after leader crash (3-node)
    // -----------------------------------------------------------------------
    {
        let mut samples = Vec::new();
        for _ in 0..runs {
            let cluster = TestCluster::new(3).await;
            let old_leader = cluster.get_leader().unwrap();
            cluster.router.block_node(old_leader).await;
            let elapsed = measure_leader_election(&cluster, old_leader).await;
            samples.push(elapsed);
            cluster.router.unblock_node(old_leader).await;
            cluster.shutdown().await;
        }
        results.push(TimingResult {
            scenario: "Re-election after leader crash (3-node)".into(),
            description: "Time from leader block to new leader elected".into(),
            samples,
        });
    }

    // -----------------------------------------------------------------------
    // 4. Re-election after leader crash (5-node)
    // -----------------------------------------------------------------------
    {
        let mut samples = Vec::new();
        for _ in 0..runs {
            let cluster = TestCluster::new(5).await;
            let old_leader = cluster.get_leader().unwrap();
            cluster.router.block_node(old_leader).await;
            let elapsed = measure_leader_election(&cluster, old_leader).await;
            samples.push(elapsed);
            cluster.router.unblock_node(old_leader).await;
            cluster.shutdown().await;
        }
        results.push(TimingResult {
            scenario: "Re-election after leader crash (5-node)".into(),
            description: "Time from leader block to new leader elected".into(),
            samples,
        });
    }

    // -----------------------------------------------------------------------
    // 5. Single write commit latency (3-node, steady state)
    // -----------------------------------------------------------------------
    {
        let cluster = TestCluster::new(3).await;
        // Warm up
        cluster.write("INSERT (:Warm {x: 0})").await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut samples = Vec::new();
        for i in 0..runs {
            let q = format!("INSERT (:Lat {{i: {}}})", i);
            samples.push(measure_write_latency(&cluster, &q).await);
        }
        cluster.shutdown().await;
        results.push(TimingResult {
            scenario: "Write commit latency (3-node)".into(),
            description: "Time for a single INSERT to commit via Raft consensus".into(),
            samples,
        });
    }

    // -----------------------------------------------------------------------
    // 6. Single write commit latency (5-node, steady state)
    // -----------------------------------------------------------------------
    {
        let cluster = TestCluster::new(5).await;
        cluster.write("INSERT (:Warm {x: 0})").await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut samples = Vec::new();
        for i in 0..runs {
            let q = format!("INSERT (:Lat5 {{i: {}}})", i);
            samples.push(measure_write_latency(&cluster, &q).await);
        }
        cluster.shutdown().await;
        results.push(TimingResult {
            scenario: "Write commit latency (5-node)".into(),
            description: "Time for a single INSERT to commit in 5-node cluster".into(),
            samples,
        });
    }

    // -----------------------------------------------------------------------
    // 7. Follower catch-up after partition (1 write missed)
    // -----------------------------------------------------------------------
    {
        let mut samples = Vec::new();
        for _ in 0..runs {
            let cluster = TestCluster::new(3).await;
            let leader = cluster.get_leader().unwrap();
            let follower = (1..=3u64).find(|&id| id != leader).unwrap();
            cluster.router.block_node(follower).await;

            cluster.write("INSERT (:Miss {x: 1})").await.unwrap();
            // Data is on leader + other follower but not on blocked one.

            cluster.router.unblock_node(follower).await;
            let elapsed = measure_replication_to(&cluster, follower, 1).await;
            samples.push(elapsed);
            cluster.shutdown().await;
        }
        results.push(TimingResult {
            scenario: "Follower catch-up (1 missed write)".into(),
            description: "Time from unblock to follower receiving missed entry".into(),
            samples,
        });
    }

    // -----------------------------------------------------------------------
    // 8. Follower catch-up after partition (50 writes missed)
    // -----------------------------------------------------------------------
    {
        let mut samples = Vec::new();
        for _ in 0..runs {
            let cluster = TestCluster::new(3).await;
            let leader = cluster.get_leader().unwrap();
            let follower = (1..=3u64).find(|&id| id != leader).unwrap();
            cluster.router.block_node(follower).await;

            for i in 0..50 {
                let q = format!("INSERT (:M50 {{i: {}}})", i);
                cluster.write(&q).await.unwrap();
            }

            cluster.router.unblock_node(follower).await;
            let elapsed = measure_replication_to(&cluster, follower, 50).await;
            samples.push(elapsed);
            cluster.shutdown().await;
        }
        results.push(TimingResult {
            scenario: "Follower catch-up (50 missed writes)".into(),
            description: "Time from unblock to follower catching up 50 entries".into(),
            samples,
        });
    }

    // -----------------------------------------------------------------------
    // 9. Follower catch-up after partition (500 writes missed)
    // -----------------------------------------------------------------------
    {
        let mut samples = Vec::new();
        for _ in 0..runs {
            let cluster = TestCluster::new(3).await;
            let leader = cluster.get_leader().unwrap();
            let follower = (1..=3u64).find(|&id| id != leader).unwrap();
            cluster.router.block_node(follower).await;

            for i in 0..500 {
                let q = format!("INSERT (:M500 {{i: {}}})", i);
                cluster.write(&q).await.unwrap();
            }

            cluster.router.unblock_node(follower).await;
            let elapsed = measure_replication_to(&cluster, follower, 500).await;
            samples.push(elapsed);
            cluster.shutdown().await;
        }
        results.push(TimingResult {
            scenario: "Follower catch-up (500 missed writes)".into(),
            description: "Time from unblock to follower catching up 500 entries".into(),
            samples,
        });
    }

    // -----------------------------------------------------------------------
    // 10. Full write availability recovery after leader crash
    // -----------------------------------------------------------------------
    {
        let mut samples = Vec::new();
        for _ in 0..runs {
            let cluster = TestCluster::new(3).await;
            let old_leader = cluster.get_leader().unwrap();
            cluster.router.block_node(old_leader).await;

            let start = Instant::now();
            // Poll until a write actually succeeds on the new leader
            loop {
                if start.elapsed() > Duration::from_secs(30) {
                    panic!("timeout waiting for write availability");
                }
                if let Some(new_leader) = cluster.get_leader() {
                    if new_leader != old_leader {
                        if let Ok(resp) = cluster.write("INSERT (:Avail {})").await {
                            if resp.success {
                                break;
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            samples.push(start.elapsed());
            cluster.router.unblock_node(old_leader).await;
            cluster.shutdown().await;
        }
        results.push(TimingResult {
            scenario: "Write availability after leader crash".into(),
            description: "Time from leader block to first successful write on new leader".into(),
            samples,
        });
    }

    // -----------------------------------------------------------------------
    // 11. Full convergence after leader crash + re-election + write
    // -----------------------------------------------------------------------
    {
        let mut samples = Vec::new();
        for _ in 0..runs {
            let cluster = TestCluster::new(3).await;
            // Pre-load some data
            for i in 0..10 {
                cluster
                    .write(&format!("INSERT (:Pre {{i: {}}})", i))
                    .await
                    .unwrap();
            }
            cluster.wait_for_convergence_count(10, 5000).await;

            let old_leader = cluster.get_leader().unwrap();
            cluster.router.block_node(old_leader).await;

            let start = Instant::now();
            // Wait for new leader and write availability
            loop {
                if start.elapsed() > Duration::from_secs(30) {
                    panic!("timeout");
                }
                if let Some(new_leader) = cluster.get_leader() {
                    if new_leader != old_leader {
                        if let Ok(resp) = cluster.write("INSERT (:Post {})").await {
                            if resp.success {
                                break;
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }

            // Now unblock old leader and wait for full convergence
            cluster.router.unblock_node(old_leader).await;
            let repl_start = Instant::now();
            measure_convergence(&cluster, 11).await;
            // Total = election + write + old leader catch-up
            samples.push(start.elapsed() + repl_start.elapsed());
            cluster.shutdown().await;
        }
        results.push(TimingResult {
            scenario: "Full recovery (crash → converge)".into(),
            description: "Leader crash → re-election → write → old leader rejoins and syncs".into(),
            samples,
        });
    }

    // -----------------------------------------------------------------------
    // 12. Network partition heal — stale leader step-down
    // -----------------------------------------------------------------------
    {
        let mut samples = Vec::new();
        for _ in 0..runs {
            let cluster = TestCluster::new(5).await;
            let old_leader = cluster.get_leader().unwrap();
            cluster.router.block_node(old_leader).await;
            measure_leader_election(&cluster, old_leader).await;

            // Heal partition — measure until old leader's metrics no longer claim leadership
            cluster.router.unblock_node(old_leader).await;
            let start = Instant::now();
            loop {
                if start.elapsed() > Duration::from_secs(15) {
                    break; // Give up, record the time
                }
                let metrics = cluster.metrics(old_leader);
                if metrics.current_leader != Some(old_leader) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            samples.push(start.elapsed());
            cluster.shutdown().await;
        }
        results.push(TimingResult {
            scenario: "Stale leader step-down after heal".into(),
            description: "Time from partition heal to old leader recognizing new leader".into(),
            samples,
        });
    }

    // -----------------------------------------------------------------------
    // 13. Learner node catch-up (join cluster with 100 existing entries)
    // -----------------------------------------------------------------------
    {
        let mut samples = Vec::new();
        for _ in 0..runs {
            let mut cluster = TestCluster::new(3).await;
            for i in 0..100 {
                cluster
                    .write(&format!("INSERT (:Existing {{i: {}}})", i))
                    .await
                    .unwrap();
            }
            cluster.wait_for_convergence_count(100, 15_000).await;

            let start = Instant::now();
            cluster.add_learner(4).await;
            measure_replication_to(&cluster, 4, 100).await;
            samples.push(start.elapsed());
            cluster.shutdown().await;
        }
        results.push(TimingResult {
            scenario: "New learner catch-up (100 entries)".into(),
            description: "Time from add_learner to new node having all 100 entries".into(),
            samples,
        });
    }

    // -----------------------------------------------------------------------
    // 14. Cluster reform after total partition (all 5 blocked/unblocked)
    // -----------------------------------------------------------------------
    {
        let mut samples = Vec::new();
        for _ in 0..runs {
            let cluster = TestCluster::new(5).await;
            for id in 1..=5u64 {
                cluster.router.block_node(id).await;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;

            for id in 1..=5u64 {
                cluster.router.unblock_node(id).await;
            }

            let start = Instant::now();
            // Wait for a leader AND successful write
            loop {
                if start.elapsed() > Duration::from_secs(30) {
                    panic!("timeout waiting for cluster reform");
                }
                if cluster.get_leader().is_some() {
                    if let Ok(resp) = cluster.write("INSERT (:Reform {})").await {
                        if resp.success {
                            break;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            samples.push(start.elapsed());
            cluster.shutdown().await;
        }
        results.push(TimingResult {
            scenario: "Cluster reform after total partition".into(),
            description: "Time from all-unblock to first successful write".into(),
            samples,
        });
    }

    // -----------------------------------------------------------------------
    // Print the report
    // -----------------------------------------------------------------------
    print_report(&results);
}
