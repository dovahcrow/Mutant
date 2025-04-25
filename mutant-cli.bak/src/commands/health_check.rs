use crate::callbacks::get::create_get_callback;
use indicatif::MultiProgress;
use log::debug;
use mutant_lib::MutAnt;
use mutant_lib::error::Error as LibError;

pub async fn run(
    mut mutant: MutAnt,
    key_name: String,
    recycle: bool,
    multi_progress: &MultiProgress,
    quiet: bool,
) -> Result<(), LibError> {
    debug!("CLI: Handling Health Check command: key_name={}", key_name);

    let (_download_pb_opt, callback) = create_get_callback(multi_progress, quiet);

    mutant.set_get_callback(callback).await;

    let result: Result<(), LibError> = match mutant.health_check(&key_name, recycle).await {
        Ok(_) => Ok(()),
        Err(e) => {
            eprintln!("Error: {:?}", e);
            Err(e)
        }
    };

    result
}
