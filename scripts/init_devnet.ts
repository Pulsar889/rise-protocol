/**
 * Devnet initialization script.
 * Initializes all collateral configs, collateral vaults, and payment configs.
 * Run with: npx ts-node scripts/init_devnet.ts
 */
import * as anchor from "@coral-xyz/anchor";
import { Program, AnchorProvider, Wallet } from "@coral-xyz/anchor";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
} from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import * as fs from "fs";
import * as path from "path";

// ── Constants ──────────────────────────────────────────────────────────────────

const RPC = "https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6";
const CDP_PROGRAM_ID = new PublicKey("3snPJTuZP9XHNciH7Q5KZzsvk2doxpuoYqWXf8JofEPR");

// Keypair path
const KEYPAIR_PATH = process.env.ANCHOR_WALLET ??
  `${process.env.HOME}/.config/solana/id.json`;

// Pyth devnet price feed accounts (used as oracle references)
const PYTH_SOL_USD  = new PublicKey("J83w4HKfqxwcq3BEMMkPFSppX3gqekLyLJBexebFVkix");
const PYTH_ETH_USD  = new PublicKey("EdVCmQ9FSPcVe5YySXDPCRmc8aDQLKJ9xvYBMZPie1Vw");
const PYTH_USDT_USD = new PublicKey("5SSkXsEKQepHHAewytPVwdej4epN1nxgLVM84L4KXgy7");

// Collateral mints (devnet)
const MINTS = {
  WSOL:    new PublicKey("So11111111111111111111111111111111111111112"),
  riseSOL: new PublicKey("86bHg3K32cRhnfcYTr3RCgKZme4xSLZzMyzWA8qDswHp"),
  mSOL:    new PublicKey("mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So"),
  JitoSOL: new PublicKey("J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn"),
  wETH:    new PublicKey("7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs"),
  wBTC:    new PublicKey("3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh"),
  USDC:    new PublicKey("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"),
  USDT:    new PublicKey("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB"),
};

// Collateral configs: [maxLtvBps, liqThreshBps, liqPenaltyBps,
//                      baseRateBps, rateSlope1Bps, rateSlope2Bps,
//                      optimalUtilBps, conversionSlippageBps, pythFeed]
// NOTE: wETH, wBTC, USDC, USDT mainnet mint addresses don't exist as SPL Token
// mints on devnet — skipped here. LST collaterals are the core use case.
type CollateralParams = [number, number, number, number, number, number, number, number, PublicKey];

const COLLATERAL_CONFIGS: Record<string, CollateralParams> = {
  WSOL:    [7500, 8500, 500, 100, 400, 3000, 8000, 50, PYTH_SOL_USD],
  riseSOL: [7800, 8700, 500, 100, 400, 3000, 8000, 50, PYTH_SOL_USD],
  mSOL:    [7800, 8700, 500, 100, 400, 3000, 8000, 50, PYTH_SOL_USD],
  JitoSOL: [7800, 8700, 500, 100, 400, 3000, 8000, 50, PYTH_SOL_USD],
};

// Payment configs: [mint, pythFeed]
// Native SOL uses SystemProgram.programId as sentinel
const PAYMENT_CONFIGS: Array<[string, PublicKey, PublicKey]> = [
  ["SOL",  SystemProgram.programId,        PYTH_SOL_USD],
  ["USDC", MINTS.USDC,                     PYTH_USDT_USD],
  ["USDT", MINTS.USDT,                     PYTH_USDT_USD],
];

// ── Helpers ───────────────────────────────────────────────────────────────────

function derivePda(seeds: Buffer[], programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(seeds, programId)[0];
}

function deriveCdpConfig() {
  return derivePda([Buffer.from("cdp_config")], CDP_PROGRAM_ID);
}

function deriveCollateralConfig(mint: PublicKey) {
  return derivePda([Buffer.from("collateral_config"), mint.toBuffer()], CDP_PROGRAM_ID);
}

function deriveCollateralVault(mint: PublicKey) {
  return derivePda([Buffer.from("collateral_vault"), mint.toBuffer()], CDP_PROGRAM_ID);
}

function derivePaymentConfig(mint: PublicKey) {
  return derivePda([Buffer.from("payment_config"), mint.toBuffer()], CDP_PROGRAM_ID);
}

async function accountExists(connection: Connection, address: PublicKey): Promise<boolean> {
  const info = await connection.getAccountInfo(address);
  return info !== null;
}

// ── Main ──────────────────────────────────────────────────────────────────────

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
  console.log("Authority:  ", payer.publicKey.toBase58());
  console.log("CDP config: ", cdpConfig.toBase58());
  console.log();

  // ── Initialize collateral configs ──────────────────────────────────────────
  for (const [symbol, params] of Object.entries(COLLATERAL_CONFIGS)) {
    const [maxLtv, liqThresh, liqPenalty, baseRate, slope1, slope2, optUtil, slippage, pythFeed] = params;
    const mint = MINTS[symbol as keyof typeof MINTS];
    const collateralConfig = deriveCollateralConfig(mint);
    const collateralVault  = deriveCollateralVault(mint);

    // initialize_collateral_config
    if (await accountExists(connection, collateralConfig)) {
      console.log(`[SKIP] collateral_config ${symbol} already exists`);
    } else {
      try {
        const sig = await program.methods
          .initializeCollateralConfig(maxLtv, liqThresh, liqPenalty, baseRate, slope1, slope2, optUtil, slippage)
          .accounts({
            authority:        payer.publicKey,
            cdpConfig,
            collateralConfig,
            collateralMint:   mint,
            pythPriceFeed:    pythFeed,
            systemProgram:    SystemProgram.programId,
          })
          .rpc();
        console.log(`[OK]   collateral_config ${symbol}: ${sig.slice(0, 20)}...`);
      } catch (e: any) {
        console.error(`[ERR]  collateral_config ${symbol}:`, e.message ?? e);
      }
    }

    // initialize_collateral_vault
    if (await accountExists(connection, collateralVault)) {
      console.log(`[SKIP] collateral_vault  ${symbol} already exists`);
    } else {
      try {
        const sig = await program.methods
          .initializeCollateralVault()
          .accounts({
            authority:        payer.publicKey,
            collateralConfig,
            collateralMint:   mint,
            collateralVault,
            tokenProgram:     TOKEN_PROGRAM_ID,
            systemProgram:    SystemProgram.programId,
            rent:             SYSVAR_RENT_PUBKEY,
          })
          .rpc();
        console.log(`[OK]   collateral_vault  ${symbol}: ${sig.slice(0, 20)}...`);
      } catch (e: any) {
        console.error(`[ERR]  collateral_vault  ${symbol}:`, e.message ?? e);
      }
    }
  }

  console.log();

  // ── Initialize payment configs ─────────────────────────────────────────────
  for (const [symbol, mint, pythFeed] of PAYMENT_CONFIGS) {
    const paymentConfig = derivePaymentConfig(mint);

    if (await accountExists(connection, paymentConfig)) {
      console.log(`[SKIP] payment_config ${symbol} already exists`);
    } else {
      try {
        const sig = await program.methods
          .initializePaymentConfig()
          .accounts({
            authority:      payer.publicKey,
            paymentConfig,
            mint,
            pythPriceFeed:  pythFeed,
            systemProgram:  SystemProgram.programId,
          })
          .rpc();
        console.log(`[OK]   payment_config ${symbol}: ${sig.slice(0, 20)}...`);
      } catch (e: any) {
        console.error(`[ERR]  payment_config ${symbol}:`, e.message ?? e);
      }
    }
  }

  console.log("\nDone.");
}

main().catch((e) => { console.error(e); process.exit(1); });
