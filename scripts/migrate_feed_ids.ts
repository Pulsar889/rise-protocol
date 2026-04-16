/**
 * One-time migration: update on-chain collateral configs and payment configs
 * to store Pyth pull-oracle feed IDs (32-byte hex) instead of the old
 * push-oracle price account pubkeys.
 *
 * Run with: npx ts-node scripts/migrate_feed_ids.ts
 */
import * as anchor from "@coral-xyz/anchor";
import { Program, AnchorProvider, Wallet } from "@coral-xyz/anchor";
import { Connection, Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import * as fs from "fs";
import * as path from "path";

const RPC = "https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6";
const CDP_PROGRAM_ID = new PublicKey("3snPJTuZP9XHNciH7Q5KZzsvk2doxpuoYqWXf8JofEPR");
const KEYPAIR_PATH = process.env.ANCHOR_WALLET ?? `${process.env.HOME}/.config/solana/id.json`;

// Pyth pull-oracle feed IDs (32-byte hex → stored as Pubkey bytes on-chain)
const FEED_SOL_USD  = new PublicKey(Buffer.from("ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d", "hex"));
const FEED_USDC_USD = new PublicKey(Buffer.from("eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a", "hex"));
const FEED_USDT_USD = new PublicKey(Buffer.from("2b89b9dc8fdf9f34709a5b106b472f0f39bb6ca9ce04b0fd7f2e971688e2e53b", "hex"));

// Collateral mints → correct feed ID
const COLLATERAL_FEEDS: Array<[string, PublicKey, PublicKey]> = [
  ["WSOL",    new PublicKey("So11111111111111111111111111111111111111112"),           FEED_SOL_USD],
  ["riseSOL", new PublicKey("86bHg3K32cRhnfcYTr3RCgKZme4xSLZzMyzWA8qDswHp"),         FEED_SOL_USD],
  ["mSOL",    new PublicKey("mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So"),         FEED_SOL_USD],
  ["JitoSOL", new PublicKey("J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn"),        FEED_SOL_USD],
];

// Payment mints → correct feed ID
const PAYMENT_FEEDS: Array<[string, PublicKey, PublicKey]> = [
  ["SOL",  SystemProgram.programId,                                                    FEED_SOL_USD],
  ["USDC", new PublicKey("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"),            FEED_USDC_USD],
  ["USDT", new PublicKey("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB"),            FEED_USDT_USD],
];

function deriveCollateralConfig(mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("collateral_config"), mint.toBuffer()], CDP_PROGRAM_ID
  )[0];
}

function derivePaymentConfig(mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("payment_config"), mint.toBuffer()], CDP_PROGRAM_ID
  )[0];
}

function deriveCdpConfig(): PublicKey {
  return PublicKey.findProgramAddressSync([Buffer.from("cdp_config")], CDP_PROGRAM_ID)[0];
}

async function main() {
  const connection = new Connection(RPC, "confirmed");
  const raw = JSON.parse(fs.readFileSync(KEYPAIR_PATH, "utf-8"));
  const payer = Keypair.fromSecretKey(Uint8Array.from(raw));
  const wallet = new Wallet(payer);
  const provider = new AnchorProvider(connection, wallet, { commitment: "confirmed" });
  anchor.setProvider(provider);

  const idlPath = path.join(__dirname, "../target/idl/rise_cdp.json");
  const idl = JSON.parse(fs.readFileSync(idlPath, "utf-8"));
  const program = new Program(idl, provider) as any;

  const cdpConfig = deriveCdpConfig();

  console.log("Authority:", payer.publicKey.toBase58());
  console.log();

  // ── Update collateral configs ─────────────────────────────────────────────
  for (const [symbol, mint, feedId] of COLLATERAL_FEEDS) {
    const collateralConfig = deriveCollateralConfig(mint);

    // Check if the account exists
    const info = await connection.getAccountInfo(collateralConfig);
    if (!info) {
      console.log(`[SKIP] collateral_config ${symbol} — account not found`);
      continue;
    }

    // Read the current stored feed ID
    const data = await program.account.collateralConfig.fetch(collateralConfig);
    const currentFeed: string = (data.pythPriceFeed as PublicKey).toBase58();
    const targetFeed = feedId.toBase58();

    if (currentFeed === targetFeed) {
      console.log(`[OK]   collateral_config ${symbol} — feed already correct`);
      continue;
    }

    console.log(`[UPD]  collateral_config ${symbol}: ${currentFeed.slice(0, 12)}... → ${targetFeed.slice(0, 12)}...`);
    try {
      const sig = await program.methods
        .updateCollateralConfig(feedId, null, null, null, null, null, null, null, null, null)
        .accounts({
          authority: payer.publicKey,
          collateralConfig,
        })
        .rpc();
      console.log(`       tx: ${sig.slice(0, 20)}...`);
    } catch (e: any) {
      console.error(`[ERR]  collateral_config ${symbol}:`, e.message ?? e);
    }
  }

  console.log();

  // ── Update payment configs ────────────────────────────────────────────────
  for (const [symbol, mint, feedId] of PAYMENT_FEEDS) {
    const paymentConfig = derivePaymentConfig(mint);

    const info = await connection.getAccountInfo(paymentConfig);
    if (!info) {
      console.log(`[SKIP] payment_config ${symbol} — account not found`);
      continue;
    }

    const data = await program.account.paymentConfig.fetch(paymentConfig);
    const currentFeed: string = (data.pythPriceFeed as PublicKey).toBase58();
    const targetFeed = feedId.toBase58();

    if (currentFeed === targetFeed) {
      console.log(`[OK]   payment_config ${symbol} — feed already correct`);
      continue;
    }

    console.log(`[UPD]  payment_config ${symbol}: ${currentFeed.slice(0, 12)}... → ${targetFeed.slice(0, 12)}...`);
    try {
      const sig = await program.methods
        .updatePaymentConfig(feedId, null)
        .accounts({
          authority:     payer.publicKey,
          cdpConfig,
          paymentConfig,
        })
        .rpc();
      console.log(`       tx: ${sig.slice(0, 20)}...`);
    } catch (e: any) {
      console.error(`[ERR]  payment_config ${symbol}:`, e.message ?? e);
    }
  }

  console.log("\nDone.");
}

main().catch((e) => { console.error(e); process.exit(1); });
