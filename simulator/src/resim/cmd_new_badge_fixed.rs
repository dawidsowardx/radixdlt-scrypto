use clap::{crate_version, App, Arg, ArgMatches};
use radix_engine::transaction::*;
use scrypto::rust::collections::HashMap;

use crate::ledger::*;
use crate::resim::*;

const ARG_SUPPLY: &str = "SUPPLY";

const ARG_TRACE: &str = "TRACE";
const ARG_SIGNERS: &str = "SIGNERS";
const ARG_SYMBOL: &str = "SYMBOL";
const ARG_NAME: &str = "NAME";
const ARG_DESCRIPTION: &str = "DESCRIPTION";
const ARG_URL: &str = "URL";
const ARG_ICON_URL: &str = "ICON_URL";

/// Constructs a `new-badge-fixed` subcommand.
pub fn make_new_badge_fixed<'a>() -> App<'a> {
    App::new(CMD_NEW_BADGE_FIXED)
        .about("Creates badge resource with fixed supply")
        .version(crate_version!())
        .arg(
            Arg::new(ARG_SUPPLY)
                .help("Specify the total supply.")
                .required(true),
        )
        // options
        .arg(Arg::new(ARG_TRACE).long("trace").help("Turn on tracing."))
        .arg(
            Arg::new(ARG_SIGNERS)
                .long("signers")
                .takes_value(true)
                .help("Specify the transaction signers, separated by comma."),
        )
        .arg(
            Arg::new(ARG_SYMBOL)
                .long("symbol")
                .takes_value(true)
                .help("Specify the symbol.")
                .required(false),
        )
        .arg(
            Arg::new(ARG_NAME)
                .long("name")
                .takes_value(true)
                .help("Specify the name.")
                .required(false),
        )
        .arg(
            Arg::new(ARG_DESCRIPTION)
                .long("description")
                .takes_value(true)
                .help("Specify the description.")
                .required(false),
        )
        .arg(
            Arg::new(ARG_URL)
                .long("url")
                .takes_value(true)
                .help("Specify the URL.")
                .required(false),
        )
        .arg(
            Arg::new(ARG_ICON_URL)
                .long("icon-url")
                .takes_value(true)
                .help("Specify the icon URL.")
                .required(false),
        )
}

/// Handles a `new-badge-fixed` request.
pub fn handle_new_badge_fixed(matches: &ArgMatches) -> Result<(), Error> {
    let supply = match_amount(matches, ARG_SUPPLY)?;

    let trace = matches.is_present(ARG_TRACE);
    let signers = match_signers(matches, ARG_SIGNERS)?;
    let mut metadata = HashMap::new();
    matches
        .value_of(ARG_SYMBOL)
        .and_then(|v| metadata.insert("symbol".to_owned(), v.to_owned()));
    matches
        .value_of(ARG_NAME)
        .and_then(|v| metadata.insert("name".to_owned(), v.to_owned()));
    matches
        .value_of(ARG_DESCRIPTION)
        .and_then(|v| metadata.insert("description".to_owned(), v.to_owned()));
    matches
        .value_of(ARG_URL)
        .and_then(|v| metadata.insert("url".to_owned(), v.to_owned()));
    matches
        .value_of(ARG_ICON_URL)
        .and_then(|v| metadata.insert("icon_url".to_owned(), v.to_owned()));

    let mut configs = get_configs()?;
    let account = configs.default_account.ok_or(Error::NoDefaultAccount)?;
    let mut ledger = FileBasedLedger::with_bootstrap(get_data_dir()?);
    let mut executor =
        TransactionExecutor::new(&mut ledger, configs.current_epoch, configs.nonce, trace);
    let transaction = TransactionBuilder::new(&executor)
        .new_badge_fixed(metadata, supply)
        .call_method_with_all_resources(account.0, "deposit_batch")
        .build(signers)
        .map_err(Error::TransactionConstructionError)?;
    let receipt = executor
        .run(transaction)
        .map_err(Error::TransactionValidationError)?;

    println!("{:?}", receipt);
    if receipt.result.is_ok() {
        configs.nonce = executor.nonce();
        set_configs(configs)?;
    }

    receipt.result.map_err(Error::TransactionExecutionError)
}
