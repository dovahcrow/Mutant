use std::fmt::Debug;

#[derive(Debug)]
pub enum PoolError<TaskError>
where
    TaskError: Debug,
{
    TaskError(TaskError),
    JoinError(tokio::task::JoinError),
    PoolSetupError(String),
    ClientAcquisitionError(String),
}
