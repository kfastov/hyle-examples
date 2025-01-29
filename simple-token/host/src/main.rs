use clap::{Parser, Subcommand};
use client_sdk::helpers::risc0::Risc0Prover;
use contract::TokenContract;
use contract::TokenContractState;
use sdk::erc20::ERC20;
use sdk::BlobTransaction;
use sdk::ProofTransaction;
use sdk::{ContractAction, RegisterContractAction};
use sdk::{ContractInput, Digestable};
use sdk::Identity;

// These constants represent the RISC-V ELF and the image ID generated by risc0-build.
// The ELF is used for proving and the ID is used for verification.
use methods::{GUEST_ELF, GUEST_ID};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[clap(long, short)]
    reproducible: bool,

    #[arg(long, default_value = "http://localhost:4321")]
    pub host: String,

    #[arg(long, default_value = "simple_token")]
    pub contract_name: String,
}

#[derive(Subcommand)]
enum Commands {
    Register {
        supply: u128,
    },
    Transfer {
        from: String,
        to: String,
        amount: u128,
    },
    Balance {
        of: String,
    },
}

#[tokio::main]
async fn main() {
    // Initialize tracing. In order to view logs, run `RUST_LOG=info cargo run`
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::filter::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    let client = client_sdk::rest_client::NodeApiHttpClient::new(cli.host).unwrap();

    let contract_name = &cli.contract_name;

    let prover = Risc0Prover::new(GUEST_ELF);

    match cli.command {
        Commands::Register { supply } => {
            // Build initial state of contract
            let initial_state =
                TokenContractState::new(supply, format!("faucet.{}", contract_name).into());
            println!("Initial state: {:?}", initial_state);

            // Send the transaction to register the contract
            let register_tx = BlobTransaction {
                identity: Identity::new("hyle.hyle"), // TODO
                blobs: vec![
                    RegisterContractAction {
                    verifier: "risc0".into(),
                    program_id: sdk::ProgramId(sdk::to_u8_array(&GUEST_ID).to_vec()),
                    state_digest: initial_state.as_digest(),
                    contract_name: contract_name.clone().into(),
                }
                .as_blob("hyle".into(), None, None)],
            };
            let res = client
                .send_tx_blob(&register_tx)
                .await
                .unwrap();

            println!("✅ Register contract tx sent. Tx hash: {}", res);
        }
        Commands::Balance { of } => {
            // Fetch the state from the node
            let state: TokenContractState = client
                .get_contract(&contract_name.clone().into())
                .await
                .unwrap()
                .state
                .into();

            println!("Balances {:?}", &state);

            let contract = TokenContract::init(state, "".into());
            let balance = contract.balance_of(&of).unwrap();
            println!("Balance of {}: {}", of, balance);
        }
        Commands::Transfer { from, to, amount } => {
            // Fetch the initial state from the node
            let initial_state: TokenContractState = client
                .get_contract(&contract_name.clone().into())
                .await
                .unwrap()
                .state
                .into();
            // ----
            // Build the blob transaction
            // ----

            let action = sdk::erc20::ERC20Action::Transfer {
                recipient: to.clone(),
                amount,
            };
            let blobs = vec![sdk::Blob {
                contract_name: contract_name.clone().into(),
                data: sdk::BlobData(
                    bincode::encode_to_vec(action, bincode::config::standard())
                        .expect("failed to encode BlobData"),
                ),
            }];
            let blob_tx = BlobTransaction {
                identity: from.clone().into(),
                blobs: blobs.clone(),
            };

            // Send the blob transaction
            let blob_tx_hash = client.send_tx_blob(&blob_tx).await.unwrap();
            println!("✅ Blob tx sent. Tx hash: {}", blob_tx_hash);

            // ----
            // Prove the state transition
            // ----

            // Build the contract input
            let inputs = ContractInput {
                initial_state: initial_state.as_digest(),
                identity: from.clone().into(),
                tx_hash: blob_tx_hash,
                private_blob: sdk::BlobData(vec![]),
                blobs: blobs.clone(),
                index: sdk::BlobIndex(0),
            };

            // Generate the zk proof
            //
            let proof = prover.prove(inputs).await.unwrap();

            let proof_tx = ProofTransaction {
                proof,
                contract_name: contract_name.clone().into(),
            };

            // Send the proof transaction
            let proof_tx_hash = client.send_tx_proof(&proof_tx).await.unwrap();
            println!("✅ Proof tx sent. Tx hash: {}", proof_tx_hash);
        }
    }
}
