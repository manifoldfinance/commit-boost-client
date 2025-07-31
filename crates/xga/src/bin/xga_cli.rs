use std::sync::Arc;

use clap::{Parser, Subcommand};
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

#[derive(Subcommand)]
enum Commands {
    /// EigenLayer integration commands
    #[command(subcommand)]
    EigenLayer(EigenLayerCommand),
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

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // Load configuration
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

    match cli.command {
        Commands::EigenLayer(cmd) => {
            handle_eigenlayer_command(cmd, &mut eigenlayer, &config_arc).await?
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
