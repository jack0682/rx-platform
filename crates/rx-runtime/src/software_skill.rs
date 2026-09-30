//! The local simulation profile uses the same bounded state-writer scheduler.
use crate::writer::{Priority, Processor};
use rx_application::software_skill::*;
use rx_domain::types::Id;
use rx_ports::{Repository, StoreError};
use serde_json::{Value, json};

pub enum Command {
    Register(Package),
    Skills,
    Submit(Start, u64),
    Runs,
    Get(Id),
    Claim(Id, u64),
    Finish(Finish, u64),
    Abandon(Id, u64),
    RegisterProcess(process::Definition),
    Processes,
    StartProcess(process::StartProcess, u64),
    ProcessRun(Id),
    ProcessRuns,
    Metrics(metrics::Window),
}
pub struct Application<R>(pub Engine<R>);
impl<R: Repository + Send + 'static> Processor for Application<R> {
    type Command = Command;
    type Reply = Value;
    type Error = StoreError;
    fn priority(command: &Command) -> Priority {
        match command {
            Command::Finish(..) | Command::Abandon(..) => Priority::Control,
            _ => Priority::Normal,
        }
    }
    fn process(&mut self, c: Command) -> Result<Value, StoreError> {
        Ok(match c {
            Command::Register(p) => json!(self.0.register(p)?),
            Command::Skills => json!(self.0.skills()?),
            Command::Submit(r, n) => json!(self.0.submit(r, n)?),
            Command::Runs => json!(self.0.runs()?),
            Command::Get(id) => json!(self.0.get(&id)?),
            Command::Claim(id, n) => json!(self.0.claim(id, n)?),
            Command::Finish(r, n) => json!(self.0.finish(r, n)?),
            Command::Abandon(id, n) => json!({"unresolved":self.0.abandon_worker(id,n)?}),
            Command::RegisterProcess(p) => json!(self.0.register_process(p)?),
            Command::Processes => json!(self.0.processes()?),
            Command::StartProcess(r, n) => json!(self.0.start_process(r, n)?),
            Command::ProcessRun(id) => json!(self.0.process_run(&id)?),
            Command::ProcessRuns => json!(self.0.process_runs()?),
            Command::Metrics(window) => json!(self.0.metrics(window)?),
        })
    }
}
