/**
 * Initializes borrow_rewards_config on devnet.
 * Safe to run if the account doesn't exist yet — skips if already initialized.
 *
 * Run with: npx ts-node scripts/init_borrow_rewards.ts
 */
import * as anchor from "@coral-xyz/anchor";
import { AnchorProvider, Program, Wallet } from "@coral-xyz/anchor";
import { Connection, Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import * as fs from "fs";
import * as path from "path";

const RPC           = "https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6";
const RISE_MINT     = new PublicKey("2TysJ9Tw5WLh7hBLmC6iZp73bm6akogYEushJEf8K49Q");
const CDP_PROGRAM_ID = new PublicKey("3snPJTuZP9XHNciH7Q5KZzsvk2doxpuoYqWXf8JofEPR");

// 1,142,308 RISE/week (6 decimals), ~1 week in slots
const EPOCH_EMISSIONS = new anchor.BN("1142308000000");
const SLOTS_PER_EPOCH = new anchor.BN("604800");

const KEYPAIR_PATH = process.env.ANCHOR_WALLET ?? `${process.env.HOME}/.config/solana/id.json`;

function pda(seeds: Buffer[], programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(seeds, programId)[0];
}

async function main() {
  const connection = new Connection(RPC, "confirmed");
  const raw        = JSON.parse(fs.readFileSync(KEYPAIR_PATH, "utf-8"));
  const payer      = Keypair.fromSecretKey(Uint8Array.from(raw));
  const wallet     = new Wallet(payer);
  const provider   = new AnchorProvider(connection, wallet, { commitment: "confirmed" });
  anchor.setProvider(provider);

  const idl = JSON.parse(fs.readFileSync(path.join(__dirname, "../target/idl/rise_cdp.json"), "utf-8"));
  const cdp = new Program(idl, provider) as any;

  const cdpConfig           = pda([Buffer.from("cdp_config")],            CDP_PROGRAM_ID);
  const borrowRewardsConfig = pda([Buffer.from("borrow_rewards_config")], CDP_PROGRAM_ID);
  const borrowRewardsVault  = pda([Buffer.from("borrow_rewards_vault")],  CDP_PROGRAM_ID);

  console.log("Authority:             ", payer.publicKey.toBase58());
  console.log("borrowRewardsConfig:   ", borrowRewardsConfig.toBase58());
  console.log("borrowRewardsVault:    ", borrowRewardsVault.toBase58());
  console.log();

  const existing = await connection.getAccountInfo(borrowRewardsConfig);
  if (existing) {
    console.log("[SKIP] borrow_rewards_config already exists — nothing to do.");
    return;
  }

  console.log("Initializing borrow_rewards_config...");
  const sig = await cdp.methods
    .initializeBorrowRewards(EPOCH_EMISSIONS, SLOTS_PER_EPOCH)
    .accounts({
      authority:          payer.publicKey,
      cdpConfig,
      borrowRewardsConfig,
      rewardsVault:       borrowRewardsVault,
      riseMint:           RISE_MINT,
      tokenProgram:       TOKEN_PROGRAM_ID,
      systemProgram:      SystemProgram.programId,
    })
    .rpc();

  console.log("[OK] initialize_borrow_rewards:", sig);
  console.log();
  console.log("Fund the vault with RISE tokens to enable borrow rewards:");
  console.log("  spl-token transfer", RISE_MINT.toBase58(), "<amount>", borrowRewardsVault.toBase58());
}

main().catch((e) => { console.error(e); process.exit(1); });
