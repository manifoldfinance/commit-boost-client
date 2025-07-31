use std::{sync::Arc, path::PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use commit_boost::prelude::*;
use eyre::Result;
use tracing::{error, info};
use xga_commitment::{
    config::XgaConfig,
    eigenlayer::{DefaultEigenLayerIntegration, EigenLayerQueries},
};

#[derive(Parser)]
#[command(name = "xga-cli")]
#[command(about = "XGA Commitment Module CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone, ValueEnum)]
enum OutputFormat {
    Json,
    Ssz,
}

#[derive(Subcommand)]
enum Commands {
    /// EigenLayer integration commands
    #[command(subcommand)]
    EigenLayer(EigenLayerCommand),
    
    /// Generate test validator registrations
    TestRegistration {
        /// Output format
        #[arg(long, value_enum, default_value = "json")]
        format: OutputFormat,
        
        /// Custom gas limit (default: 30000000)
        #[arg(long)]
        gas_limit: Option<u64>,
        
        /// Custom timestamp (default: current)
        #[arg(long)]
        timestamp: Option<u64>,
        
        /// Output file path (default: stdout)
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum EigenLayerCommand {
    /// Check operator shadow mode status
    Status {
        /// Operator address to check
        #[arg(long)]
        operator_address: String,
    },

    /// Update commitment tracking (for consolidation/increases)
    UpdateCommitment {
        /// Operator address
        #[arg(long)]
        operator_address: String,

        /// Validator public keys (comma-separated hex strings)
        #[arg(long, value_delimiter = ',')]
        validators: Vec<String>,
    },
}

// Test constants
const TEST_VALIDATOR_PUBKEY: &str = "0xb060572f535ba5615b874ebfef757fbe6825352ad257e31d724e57fe25a067a13cfddd0f00cb17bf3a3d2e901a380c17";
const TEST_FEE_RECIPIENT: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DEFAULT_GAS_LIMIT: u64 = 30_000_000;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::EigenLayer(cmd) => {
            // Load configuration for EigenLayer commands
            let config = load_commit_module_config::<XgaConfig>()?;

            // Ensure EigenLayer is enabled
            if !config.extra.eigenlayer.enabled {
                error!("EigenLayer integration is not enabled in configuration");
                return Err(eyre::eyre!("EigenLayer not enabled"));
            }

            // Create config arc first
            let config_arc = Arc::new(config);

            // Create EigenLayer integration with commit boost config
            let mut eigenlayer =
                DefaultEigenLayerIntegration::new(config_arc.extra.eigenlayer.clone(), config_arc.clone())
                    .await?;

            handle_eigenlayer_command(cmd, &mut eigenlayer, &config_arc).await?
        }
        Commands::TestRegistration { format, gas_limit, timestamp, output } => {
            handle_test_registration(format, gas_limit, timestamp, output).await?
        }
    }

    Ok(())
}

async fn handle_eigenlayer_command(
    cmd: EigenLayerCommand,
    eigenlayer: &mut DefaultEigenLayerIntegration,
    config: &Arc<StartCommitModuleConfig<XgaConfig>>,
) -> Result<()> {
    use alloy::primitives::Address;
    use xga_commitment::commitment::{XgaCommitment, XgaParameters};

    match cmd {
        EigenLayerCommand::Status { operator_address } => {
            info!("Checking operator shadow mode status...");

            let operator_addr: Address =
                operator_address.parse().map_err(|_| eyre::eyre!("Invalid operator address"))?;

            // Use the shadow mode status method to get comprehensive status
            let shadow_status = eigenlayer.get_shadow_mode_status(operator_addr).await?;

            // Also get detailed operator status
            let (
                _commitment_hash,
                _signature,
                registration_block,
                last_reward_block,
                is_active,
                accumulated_rewards,
            ) = eigenlayer.get_operator_status(operator_addr).await?;

            println!("=== XGA Operator Shadow Mode Status ===");
            println!("Operator: {}", operator_address);
            println!("Tracked: {}", shadow_status.is_registered);
            println!("Active: {}", is_active);
            println!("Commitment Hash: 0x{}", hex::encode(shadow_status.commitment_hash));
            println!("Registration Block: {}", registration_block);
            println!("Last Reward Block: {}", last_reward_block);
            println!("Blocks Active: {}", shadow_status.blocks_active);
            println!("Penalty Rate: {}", shadow_status.penalty_rate);
            println!(
                "Accumulated Rewards (not claimable): {} ETH",
                format_ether(accumulated_rewards)
            );
            println!(
                "Pending Rewards (not claimable): {} ETH",
                format_ether(shadow_status.pending_rewards)
            );
            println!("\nNote: Shadow mode - rewards are tracked but cannot be claimed");
        }

        EigenLayerCommand::UpdateCommitment { operator_address, validators } => {
            info!("Updating commitment tracking with {} validators", validators.len());

            let operator_addr: Address =
                operator_address.parse().map_err(|_| eyre::eyre!("Invalid operator address"))?;

            // Check current rewards before update
            let rewards_before = eigenlayer.get_pending_rewards(operator_addr).await?;

            // Parse validator public keys
            let mut pubkeys = Vec::with_capacity(validators.len());
            for v in validators {
                let bytes = hex::decode(v.trim_start_matches("0x"))
                    .map_err(|e| eyre::eyre!("Invalid validator pubkey: {}", e))?;
                if bytes.len() != 48 {
                    return Err(eyre::eyre!("Validator pubkey must be 48 bytes"));
                }
                let mut pubkey_array = [0u8; 48];
                pubkey_array.copy_from_slice(&bytes);
                pubkeys.push(BlsPublicKey::from(pubkey_array));
            }

            // Create commitment for tracking
            let commitment = XgaCommitment::new(
                [0u8; 32],  // Would be actual registration hash from relay
                pubkeys[0], // Using first validator for simplicity
                "manual-update",
                config.chain.id(),
                XgaParameters::default(),
            );

            eigenlayer.update_commitment(&commitment, pubkeys[0]).await?;

            // Check rewards after update to show change
            let rewards_after = eigenlayer.get_pending_rewards(operator_addr).await?;

            println!("✓ Commitment tracking updated successfully in shadow mode");
            println!("Operator: {}", operator_address);
            println!("Validators tracked: {}", pubkeys.len());
            println!("Pending rewards before: {} ETH", format_ether(rewards_before));
            println!("Pending rewards after: {} ETH", format_ether(rewards_after));
            println!("\nNote: This only updates local tracking - no on-chain transaction was made");
        }
    }

    Ok(())
}

fn format_ether(wei: alloy::primitives::U256) -> String {
    // Simple formatting - divide by 10^18
    let ether = wei / alloy::primitives::U256::from(10u64.pow(18));
    let remainder = wei % alloy::primitives::U256::from(10u64.pow(18));

    // Get first 4 decimal places
    let decimals = remainder / alloy::primitives::U256::from(10u64.pow(14));

    format!("{}.{:04}", ether, decimals)
}

async fn handle_test_registration(
    format: OutputFormat,
    gas_limit: Option<u64>,
    timestamp: Option<u64>,
    output: Option<PathBuf>,
) -> Result<()> {
    use alloy_rpc_types::beacon::relay::{ValidatorRegistration, ValidatorRegistrationMessage};
    use alloy_rpc_types::beacon::{BlsPublicKey as AlloyBlsPublicKey, BlsSignature as AlloyBlsSignature};
    use std::fs::File;
    use std::io::Write;
    use ssz::Encode;
    use ssz_derive::Encode as SszEncode;
    
    info!("Generating test validator registration");
    
    // Parse test validator pubkey
    let pubkey_bytes = hex::decode(TEST_VALIDATOR_PUBKEY.trim_start_matches("0x"))
        .map_err(|e| eyre::eyre!("Invalid test pubkey: {}", e))?;
    if pubkey_bytes.len() != 48 {
        return Err(eyre::eyre!("Test pubkey must be 48 bytes"));
    }
    let mut pubkey_array = [0u8; 48];
    pubkey_array.copy_from_slice(&pubkey_bytes);
    
    // Parse fee recipient
    let fee_recipient = TEST_FEE_RECIPIENT.parse::<alloy::primitives::Address>()
        .map_err(|_| eyre::eyre!("Invalid fee recipient"))?;
    
    // Get timestamp
    let registration_timestamp = timestamp.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    });
    
    // Create registration message
    let message = ValidatorRegistrationMessage {
        pubkey: AlloyBlsPublicKey::from(pubkey_array),
        fee_recipient,
        gas_limit: gas_limit.unwrap_or(DEFAULT_GAS_LIMIT),
        timestamp: registration_timestamp,
    };
    
    // Create unsigned registration (zeroed signature for now)
    let registration = ValidatorRegistration {
        message: message.clone(),
        signature: AlloyBlsSignature::from([0u8; 96]),
    };
    
    // Generate output based on format
    let output_data = match format {
        OutputFormat::Json => {
            // Create JSON output
            serde_json::to_vec_pretty(&registration)?
        }
        OutputFormat::Ssz => {
            // Create SSZ-encodable version
            #[derive(SszEncode)]
            struct SszValidatorRegistration {
                pubkey: [u8; 48],
                fee_recipient: [u8; 20],
                gas_limit: u64,
                timestamp: u64,
                signature: [u8; 96],
            }
            
            let ssz_registration = SszValidatorRegistration {
                pubkey: message.pubkey.0,
                fee_recipient: message.fee_recipient.0.into(),
                gas_limit: message.gas_limit,
                timestamp: message.timestamp,
                signature: [0u8; 96],
            };
            
            ssz_registration.as_ssz_bytes()
        }
    };
    
    // Write output
    match &output {
        Some(path) => {
            let mut file = File::create(path)?;
            file.write_all(&output_data)?;
            info!("Written test registration to {:?}", path);
        }
        None => {
            match format {
                OutputFormat::Json => {
                    println!("{}", String::from_utf8_lossy(&output_data));
                }
                OutputFormat::Ssz => {
                    println!("0x{}", hex::encode(&output_data));
                }
            }
        }
    }
    
    info!("Test registration generated successfully");
    println!("\nGenerated unsigned test registration:");
    println!("  Validator: {}", TEST_VALIDATOR_PUBKEY);
    println!("  Fee Recipient: {}", TEST_FEE_RECIPIENT);
    println!("  Gas Limit: {}", gas_limit.unwrap_or(DEFAULT_GAS_LIMIT));
    println!("  Timestamp: {}", registration_timestamp);
    println!("  Format: {:?}", format);
    if let Some(path) = output {
        println!("  Output: {:?}", path);
    }
    
    Ok(())
}
