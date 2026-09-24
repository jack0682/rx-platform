//! Isolated single-thread Linux measurement, not a product executable.
//! Unsafe code is limited to controlled fork/wait/signal and a pre-exec barrier.
use rx_domain::types::*;
use rx_ports::{Document, OwnershipFailure, Repository, StoreError};
use rx_storage::{ExclusiveFileLock, SqliteRepository};
use std::{
    io::{Read, Write},
    os::unix::{net::UnixStream, process::CommandExt},
    path::Path,
    time::{Duration, Instant},
};
struct Child {
    pid: libc::pid_t,
    reaped: bool,
}
impl Child {
    fn wait(&mut self) {
        let mut status = 0;
        assert_eq!(unsafe { libc::waitpid(self.pid, &mut status, 0) }, self.pid);
        self.reaped = true;
        assert!(
            libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
            "child status {status}"
        );
    }
    fn kill_and_wait(&mut self) {
        assert_eq!(unsafe { libc::kill(self.pid, libc::SIGKILL) }, 0);
        let mut s = 0;
        assert_eq!(unsafe { libc::waitpid(self.pid, &mut s, 0) }, self.pid);
        self.reaped = true;
        assert!(libc::WIFSIGNALED(s));
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        if !self.reaped {
            unsafe {
                libc::kill(self.pid, libc::SIGKILL);
                libc::waitpid(self.pid, std::ptr::null_mut(), 0);
            }
        }
    }
}
fn fork(f: impl FnOnce()) -> Child {
    match unsafe { libc::fork() } {
        -1 => panic!("fork: {}", std::io::Error::last_os_error()),
        0 => {
            f();
            std::process::exit(0)
        }
        pid => Child { pid, reaped: false },
    }
}
fn wait_file(path: &Path) {
    let end = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(Instant::now() < end, "timeout {}", path.display());
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn touch(path: &Path) {
    std::fs::write(path, b"ready").unwrap();
}
fn refused(path: &Path) {
    assert!(
        matches!(SqliteRepository::open(path),Err(StoreError::Ownership(e)) if e.kind==OwnershipFailure::Contended)
    );
}
fn foreign<T>(result: Result<T, StoreError>) {
    assert!(
        matches!(result,Err(StoreError::Ownership(e)) if e.kind==OwnershipFailure::ForeignProcess)
    );
}
fn record(store: &mut SqliteRepository) {
    store
        .transact(|tx| {
            tx.put(
                &Name::new("probe/value").unwrap(),
                None,
                &Document {
                    schema: Name::new("probe.value.v1").unwrap(),
                    value: serde_json::json!({"committed":true}),
                },
            )?;
            Ok(())
        })
        .unwrap();
}
fn main() {
    assert_eq!(std::env::consts::OS, "linux");
    // Fixture-only descendant reaping after intentionally killing our own manager.
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) },
        0
    );
    let mut scenes = vec![];
    for explicit in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.db");
        let mut owner = Some(SqliteRepository::open(&path).unwrap());
        record(owner.as_mut().unwrap());
        let ready = root.path().join("ready");
        let finish = root.path().join("finish");
        let mut child = fork(|| {
            touch(&ready);
            wait_file(&finish);
        });
        wait_file(&ready);
        refused(&path);
        if explicit {
            owner.take().unwrap().close().unwrap();
        } else {
            drop(owner.take());
        }
        let mut next = SqliteRepository::open(&path).unwrap();
        assert_eq!(next.snapshot().unwrap().1.len(), 1);
        refused(&path);
        touch(&finish);
        child.wait();
        refused(&path);
        next.close().unwrap();
        assert!(SqliteRepository::open(&path).is_ok());
        scenes.push(serde_json::json!({"scene":if explicit{"explicit_close_with_live_fork_copy"}else{"drop_with_live_fork_copy"},"live_writer_refused":true,"reacquired_before_child_exit":true,"new_writer_refused_after_old_child_exit":true}));
    }
    for explicit in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.db");
        let mut owner = Some(SqliteRepository::open(&path).unwrap());
        let ready = root.path().join("refused");
        let finish = root.path().join("finish");
        let mut child = fork(|| {
            foreign(owner.as_mut().unwrap().snapshot());
            foreign(
                owner
                    .as_mut()
                    .unwrap()
                    .transact::<()>(|_| panic!("inherited callback must not run")),
            );
            if explicit {
                foreign(owner.take().unwrap().close());
            } else {
                drop(owner.take());
            }
            refused(&path);
            touch(&ready);
            wait_file(&finish);
        });
        wait_file(&ready);
        refused(&path);
        record(owner.as_mut().unwrap());
        touch(&finish);
        child.wait();
        refused(&path);
        owner.take().unwrap().close().unwrap();
        assert!(SqliteRepository::open(&path).is_ok());
        scenes.push(serde_json::json!({"scene":if explicit{"descendant_close_refused"}else{"descendant_drop_does_not_unlock"},"inherited_sqlite_use_refused":true,"parent_remains_writer":true}));
    }
    // The same source-owned guard used for Host has the same opposite-direction test.
    {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("host.lock");
        let mut owner = Some(ExclusiveFileLock::acquire(&path).unwrap());
        let ready = root.path().join("refused");
        let finish = root.path().join("finish");
        let mut child = fork(|| {
            assert!(
                matches!(owner.take().unwrap().close(),Err(e) if e.kind==OwnershipFailure::ForeignProcess)
            );
            touch(&ready);
            wait_file(&finish);
        });
        wait_file(&ready);
        assert!(
            matches!(ExclusiveFileLock::acquire(&path),Err(e) if e.kind==OwnershipFailure::Contended)
        );
        owner.take().unwrap().close().unwrap();
        let next = ExclusiveFileLock::acquire(&path).unwrap();
        touch(&finish);
        child.wait();
        assert!(
            matches!(ExclusiveFileLock::acquire(&path),Err(e) if e.kind==OwnershipFailure::Contended)
        );
        next.close().unwrap();
        scenes.push(serde_json::json!({"scene":"host_guard_descendant_release_refused","live_holder_denied":true,"owner_release_reacquired":true}));
    }
    // SIGKILL never executes Drop. Retained fork copies must continue refusing entry.
    {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.db");
        let ready = root.path().join("holder");
        let finish = root.path().join("finish");
        let mut manager = fork(|| {
            let mut store = SqliteRepository::open(&path).unwrap();
            record(&mut store);
            let grandchild = fork(|| {
                std::fs::write(&ready, std::process::id().to_string()).unwrap();
                wait_file(&finish);
            });
            std::mem::forget(grandchild);
            loop {
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        wait_file(&ready);
        refused(&path);
        let pid: libc::pid_t = std::fs::read_to_string(&ready).unwrap().parse().unwrap();
        manager.kill_and_wait();
        refused(&path);
        let mut inherited = Child { pid, reaped: false };
        touch(&finish);
        inherited.wait();
        let mut reopened = SqliteRepository::open(&path).unwrap();
        assert_eq!(reopened.snapshot().unwrap().1.len(), 1);
        refused(&path);
        drop(reopened);
        scenes.push(serde_json::json!({"scene":"manager_sigkill_before_exec","while_inherited_fd_alive":"REFUSED_UNTIL_INHERITED_DESCRIPTION_CLOSES","after_child_exit":"REACQUIRED","past_work_outcome":"NOT_ASSERTED"}));
    }
    // A borrowed Transaction is also copied by fork. Child Ok/Err/unwind must
    // neither execute SQL nor commit/roll back the parent's live transaction.
    for mode in ["ok", "error", "panic"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.db");
        let ready = root.path().join("child-refused");
        let finish = root.path().join("finish");
        let creator = std::process::id();
        let mut child_pid = 0;
        let mut owner = Some(SqliteRepository::open(&path).unwrap());
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            owner.as_mut().unwrap().transact(|tx| {
                let key = Name::new("parent/committed").unwrap();
                let doc = Document {
                    schema: Name::new("probe.value.v1").unwrap(),
                    value: serde_json::json!({"parent":true}),
                };
                tx.put(&key, None, &doc)?;
                let pid = unsafe { libc::fork() };
                assert!(pid >= 0);
                if pid == 0 {
                    foreign(tx.get(&key));
                    foreign(tx.scan("parent/"));
                    foreign(tx.control_head());
                    foreign(tx.put(&Name::new("child/forbidden").unwrap(), None, &doc));
                    match mode {
                        "panic" => panic!("injected foreign transaction unwind"),
                        "error" => {
                            return Err(StoreError::Unavailable(
                                "injected foreign callback error".into(),
                            ));
                        }
                        _ => return Ok(()),
                    }
                }
                child_pid = pid;
                wait_file(&ready);
                refused(&path);
                assert!(tx.get(&key)?.is_some());
                Ok(())
            })
        }));
        if std::process::id() != creator {
            if mode == "panic" {
                assert!(outcome.is_err());
            } else {
                foreign(outcome.unwrap());
            }
            foreign(owner.take().unwrap().close());
            touch(&ready);
            wait_file(&finish);
            std::process::exit(0);
        }
        outcome.unwrap().unwrap();
        let mut child = Child {
            pid: child_pid,
            reaped: false,
        };
        touch(&finish);
        child.wait();
        assert_eq!(owner.as_mut().unwrap().snapshot().unwrap().1.len(), 1);
        owner.as_ref().unwrap().check_integrity().unwrap();
        refused(&path);
        owner.take().unwrap().close().unwrap();
        assert!(SqliteRepository::open(&path).is_ok());
        scenes.push(serde_json::json!({"scene":"fork_inside_active_transaction","child_callback":mode,"child_sql_commit_and_rollback_refused":true,"parent_transaction_preserved":true}));
    }
    // Real Command::spawn window, with no intentional passing of the lock FD.
    for executable in ["/bin/true", "/missing/g1-child"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.db");
        let owner = SqliteRepository::open(&path).unwrap();
        let (mut parent, mut child) = UnixStream::pair().unwrap();
        let thread = std::thread::spawn(move || {
            let mut command = std::process::Command::new(executable);
            // Only read/write syscalls run in the forked pre-exec closure.
            unsafe {
                command.pre_exec(move || {
                    child.write_all(b"R")?;
                    let mut b = [0];
                    child.read_exact(&mut b)?;
                    Ok(())
                });
            }
            command.spawn()
        });
        let mut b = [0];
        parent.read_exact(&mut b).unwrap();
        refused(&path);
        owner.close().unwrap();
        let next = SqliteRepository::open(&path).unwrap();
        refused(&path);
        parent.write_all(b"X").unwrap();
        let result = thread.join().unwrap();
        if executable == "/bin/true" {
            assert!(result.unwrap().wait().unwrap().success());
        } else {
            assert!(result.is_err());
        }
        refused(&path);
        next.close().unwrap();
        assert!(SqliteRepository::open(&path).is_ok());
        scenes.push(serde_json::json!({"scene":"actual_command_pre_exec","executable":executable,"live_owner_denied":true,"legitimate_close_reacquired_in_window":true,"child_completion_does_not_unlock_new_writer":true}));
    }
    println!(
        "{}",
        serde_json::json!({"result":"PASS_NORMAL_RELEASE_AND_LIVE_WRITER_EXCLUSION","abrupt_loss":"REFUSED_UNTIL_INHERITED_DESCRIPTION_CLOSES","scenes":scenes,"scope":"actual local Linux cooperating-process ownership; no process adoption, remote filesystem or physical guarantee"})
    );
}
