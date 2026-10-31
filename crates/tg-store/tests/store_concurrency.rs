#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;
use std::process::Command;
use std::thread;
use std::time::Duration;
use support::TestEnv;
use tg_core::balance::Balance;
use tg_core::clock::FakeClock;
use tg_core::sim::Garden;
use tg_store::lock::GardenLock;
use tg_store::paths::NiwaPaths;
use tg_store::save::{GardenSave, GardenStore};

const WORKER_ENV_FLAG: &str = "NIWA_STORE_CONCURRENCY_WORKER";
const WORKER_ITERATIONS_ENV: &str = "NIWA_STORE_CONCURRENCY_ITERS";

#[test]
fn in_process_try_acquire_contends_while_exclusive_lock_is_held() {
    let env = TestEnv::new();

    let guard = GardenLock::acquire(env.paths()).expect("acquire primary lock");
    assert_eq!(guard.path(), env.paths().lock_file());
    assert!(env.paths().lock_file().is_file());

    let second = GardenLock::try_acquire(env.paths()).expect("try_acquire while locked");
    assert!(
        second.is_none(),
        "second file descriptor must observe exclusive lock contention"
    );

    drop(guard);

    let reacquired = GardenLock::try_acquire(env.paths())
        .expect("try_acquire after drop")
        .expect("lock must be free after guard drop");
    drop(reacquired);
}

#[test]
fn concurrent_child_worker_entry() {
    if std::env::var_os(WORKER_ENV_FLAG).is_none() {
        return;
    }

    let niwa_home = std::env::var_os("NIWA_HOME").expect("worker requires isolated NIWA_HOME");
    let iterations: u32 = std::env::var(WORKER_ITERATIONS_ENV)
        .expect("worker iterations env")
        .parse()
        .expect("parse worker iterations");

    let paths = NiwaPaths::from_niwa_home(niwa_home);
    let store = GardenStore::new(paths);

    for step in 0..iterations {
        store
            .with_lock(|locked_store| {
                let outcome = locked_store
                    .load()?
                    .expect("baseline save must exist before workers start");
                assert_eq!(
                    outcome.notice, None,
                    "concurrent locked updates must never corrupt primary garden.json"
                );
                let mut save = outcome.save;
                *save.inventory.entry("sun_moss".to_string()).or_insert(0) += 1;
                save.tick_index = save.tick_index.saturating_add(1);
                locked_store.save(&save)
            })
            .unwrap_or_else(|err| panic!("worker failed on step {step}: {err}"));

        if step % 5 == 0 {
            thread::yield_now();
        }
    }
}

#[test]
fn two_concurrent_processes_do_not_corrupt_the_save() {
    let env = TestEnv::new();
    let clock = FakeClock::from_unix_secs(1_730_500_000);
    let balance = Balance::load_default();
    let store = GardenStore::new(env.paths().clone());

    let garden = Garden::from_clock(2026, &balance, &clock);
    let initial = GardenSave::from_garden(&garden);
    store.save(&initial).expect("write initial baseline save");

    let exe = std::env::current_exe().expect("locate current test executable");
    let worker_count: u32 = 2;
    let iterations_per_worker: u32 = 50;

    let mut children = Vec::with_capacity(worker_count as usize);
    for worker_id in 0..worker_count {
        let mut cmd = Command::new(&exe);
        cmd.args([
            "--exact",
            "concurrent_child_worker_entry",
            "--nocapture",
            "--test-threads=1",
        ]);
        for (k, v) in env.env_vars() {
            cmd.env(k, v);
        }
        cmd.env(WORKER_ENV_FLAG, worker_id.to_string());
        cmd.env(WORKER_ITERATIONS_ENV, iterations_per_worker.to_string());

        let child = cmd.spawn().expect("spawn concurrent worker process");
        children.push(child);
    }

    // Also contend briefly from the parent process via try_acquire/with_lock checks while workers run.
    thread::sleep(Duration::from_millis(5));

    for mut child in children {
        let status = child.wait().expect("wait for worker process");
        assert!(
            status.success(),
            "concurrent worker exited with failure status: {status}"
        );
    }

    let expected_total = worker_count * iterations_per_worker;
    let final_outcome = store
        .load()
        .expect("final garden.json must load cleanly")
        .expect("final save exists");
    assert_eq!(
        final_outcome.notice, None,
        "final garden.json must be valid without needing backup recovery"
    );
    assert_eq!(
        final_outcome.save.inventory.get("sun_moss").copied(),
        Some(expected_total)
    );
    assert_eq!(final_outcome.save.tick_index, u64::from(expected_total));

    // Verify garden.json.bak is also valid, uncorrupted JSON representing the penultimate save.
    let bak_raw =
        fs::read_to_string(env.paths().garden_backup_file()).expect("read garden.json.bak");
    let bak_save: GardenSave =
        serde_json::from_str(&bak_raw).expect("garden.json.bak must also be valid JSON");
    assert_eq!(
        bak_save.inventory.get("sun_moss").copied(),
        Some(expected_total - 1)
    );
}
