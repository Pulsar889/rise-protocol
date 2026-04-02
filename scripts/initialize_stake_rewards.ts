/**
 * Initialize RISE staking rewards on the rise-staking program.
 *
 * Run with: npx ts-node scripts/initialize_stake_rewards.ts
 *
 * Fund the resulting stake_rewards_vault with RISE tokens after running this.
 */
import * as anchor from "@coral-xyz/anchor";
import { AnchorProvider, Program, Wallet } from "@coral-xyz/anchor";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
} from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import * as fs from "fs";
import * as path from "path";

// ── Config ────────────────────────────────────────────────────────────────────

const RPC            = "https://devnet.helius-rpc.com/?api-key=787be2ec-9299-40c2-af00-e559a4715fa1";
const RISE_MINT      = new PublicKey("2TysJ9Tw5WLh7hBLmC6iZp73bm6akogYEushJEf8K49Q");
const STAKING_PROGRAM_ID = new PublicKey("BnQc6jJMT6mt3mvWuQFAd9vf2T2wWkAYD2uGjCXud6Lo");

// Staker share: 40% of total weekly ~3,807,692 RISE = 1,523,077 RISE/week (6 decimals)
const EPOCH_EMISSIONS  = BigInt("1523077000000");
const SLOTS_PER_EPOCH  = BigInt("604800"); // ~1 week

const KEYPAIR_PATH = process.env.ANCHOR_WALLET ?? `${process.env.HOME}/.config/solana/id.json`;

// ── PDA helpers ───────────────────────────────────────────────────────────────

function pda(seeds: Buffer[], programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(seeds, programId)[0];
}

const globalPool           = pda([Buffer.from("global_pool")],          STAKING_PROGRAM_ID);
const stakeRewardsConfig   = pda([Buffer.from("stake_rewards_config")], STAKING_PROGRAM_ID);
const stakeRewardsVault    = pda([Buffer.from("stake_rewards_vault")],  STAKING_PROGRAM_ID);

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  const connection = new Connection(RPC, "confirmed");
  const raw        = JSON.parse(fs.readFileSync(KEYPAIR_PATH, "utf-8"));
  const payer      = Keypair.fromSecretKey(Uint8Array.from(raw));
  const wallet     = new Wallet(payer);
  const provider   = new AnchorProvider(connection, wallet, { commitment: "confirmed" });
  anchor.setProvider(provider);

  const stakingIdl = JSON.parse(
    fs.readFileSync(path.join(__dirname, "../target/idl/rise_staking.json"), "utf-8")
  );
  const staking = new Program(stakingIdl, provider) as any;

  console.log("Authority:", payer.publicKey.toBase58());
  console.log("RISE mint:", RISE_MINT.toBase58());
  console.log();

  // ── Check if already initialized ─────────────────────────────────────────
  const existing = await connection.getAccountInfo(stakeRewardsConfig);
  if (existing) {
    console.log("[SKIP] stake_rewards_config already exists:", stakeRewardsConfig.toBase58());
    console.log("To re-initialize, close it first (no close instruction exists yet — use a migration script).");
    return;
  }

  console.log("Initializing stake rewards...");
  console.log(`  epoch_emissions: ${EPOCH_EMISSIONS} (${Number(EPOCH_EMISSIONS) / 1e6} RISE/epoch)`);
  console.log(`  slots_per_epoch: ${SLOTS_PER_EPOCH}`);

  const sig = await staking.methods
    .initializeStakeRewards(
      new anchor.BN(EPOCH_EMISSIONS.toString()),
      new anchor.BN(SLOTS_PER_EPOCH.toString()),
    )
    .accounts({
      authority:          payer.publicKey,
      pool:               globalPool,
      stakeRewardsConfig,
      rewardsVault:       stakeRewardsVault,
      riseMint:           RISE_MINT,
      tokenProgram:       TOKEN_PROGRAM_ID,
      systemProgram:      SystemProgram.programId,
    })
    .rpc();

  console.log("[OK] initialize_stake_rewards:", sig.slice(0, 20) + "...");
  console.log();
  console.log("stake_rewards_config: ", stakeRewardsConfig.toBase58());
  console.log("stake_rewards_vault:  ", stakeRewardsVault.toBase58());
  console.log();
  console.log("Next: fund stake_rewards_vault with RISE tokens.");
}

main().catch((e) => { console.error(e); process.exit(1); });
