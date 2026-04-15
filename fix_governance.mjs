/**
 * fix_governance.mjs
 *
 * Fixes the broken governance setup on devnet.
 * The governance_config has the wrong rise_mint and the rise_vault was
 * initialised with yet another wrong mint.  This script:
 *
 *   1. Reads on-chain state so you can see what will happen before anything is
 *      sent.
 *   2. Closes rise_vault (burns the worthless wrong-mint tokens, reclaims rent)
 *   3. Closes governance_config (reclaims rent)
 *   4. Re-initialises governance_config with the real RISE mint
 *   5. Re-initialises rise_vault with the new (correct) config
 *
 * Your RISE tokens (2TysJ9...) are NEVER moved or touched.
 *
 * Usage:
 *   node fix_governance.mjs          # dry-run: prints what will happen, no tx
 *   node fix_governance.mjs --exec   # executes all steps
 */

import { Connection, PublicKey, Keypair, SystemProgram } from "@solana/web3.js";
import pkg from "@coral-xyz/anchor";
const { AnchorProvider, Program, Wallet, BN } = pkg;
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { execSync } from "child_process";
import { readFileSync } from "fs";
import { homedir } from "os";
import { join } from "path";
import { createRequire } from "module";

// ─── Config ────────────────────────────────────────────────────────────────
const RPC            = "https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6";
const GOV_PROGRAM_ID = new PublicKey("CtMKhgY5xKiwLB5jmQ44PRF9QsUqXqSbiyVbFsidskHz");
const REAL_RISE_MINT = new PublicKey("2TysJ9Tw5WLh7hBLmC6iZp73bm6akogYEushJEf8K49Q");

// Governance init params (same as original setup)
const PROPOSAL_THRESHOLD = new BN("10000000000"); // 10,000 RISE (6 decimals)
const QUORUM_BPS         = 500; // 5%

const DRY_RUN = !process.argv.includes("--exec");

// ─── Load wallet (same keypair the solana CLI uses) ─────────────────────────
const walletPath = execSync("solana config get keypair").toString()
  .match(/Keypair Path: (.+)/)?.[1]?.trim()
  ?? join(homedir(), ".config/solana/id.json");

const secret  = JSON.parse(readFileSync(walletPath, "utf8"));
const keypair = Keypair.fromSecretKey(Uint8Array.from(secret));
console.log("Authority:", keypair.publicKey.toBase58());

// ─── Anchor setup ───────────────────────────────────────────────────────────
const connection = new Connection(RPC, "confirmed");
const wallet     = new Wallet(keypair);
const provider   = new AnchorProvider(connection, wallet, { commitment: "confirmed" });

const require = createRequire(import.meta.url);
const idl     = require("./target/idl/rise_governance.json");
const program = new Program(idl, provider);

// ─── PDAs ───────────────────────────────────────────────────────────────────
const [configPda] = PublicKey.findProgramAddressSync([Buffer.from("governance_config")], GOV_PROGRAM_ID);
const [vaultPda]  = PublicKey.findProgramAddressSync([Buffer.from("rise_vault")],        GOV_PROGRAM_ID);

console.log("governance_config PDA:", configPda.toBase58());
console.log("rise_vault PDA:       ", vaultPda.toBase58());
console.log("Real RISE mint:       ", REAL_RISE_MINT.toBase58());
console.log();

// ─── Read current on-chain state ────────────────────────────────────────────
const [cfgInfo, vaultInfo] = await Promise.all([
  connection.getAccountInfo(configPda),
  connection.getAccountInfo(vaultPda),
]);

if (!cfgInfo)   console.log("governance_config: NOT FOUND (already closed)");
if (!vaultInfo) console.log("rise_vault:        NOT FOUND (already closed)");

let wrongMint = null;
let configAlreadyCorrect = false;

if (cfgInfo) {
  // rise_mint is at bytes [40,72): 8 disc + 32 authority
  const mintInConfig = new PublicKey(cfgInfo.data.slice(40, 72)).toBase58();
  console.log("Current config.rise_mint:", mintInConfig);
  configAlreadyCorrect = mintInConfig === REAL_RISE_MINT.toBase58();
  if (configAlreadyCorrect) console.log("  ✓ config already has the correct RISE mint — will not touch it");
}
if (vaultInfo) {
  // SPL token account layout: mint = bytes [0,32), amount = bytes [64,72)
  wrongMint = new PublicKey(vaultInfo.data.slice(0, 32));
  const balanceBig = vaultInfo.data.readBigUInt64LE(64);
  console.log("Current vault.mint:      ", wrongMint.toBase58());
  console.log("Current vault.balance:   ", balanceBig.toString(), "tokens (wrong mint, will be burned)");
}
console.log();

const needCloseConfig = cfgInfo && !configAlreadyCorrect;
const needInitConfig  = !cfgInfo || !configAlreadyCorrect;
const needCloseVault  = vaultInfo && wrongMint && wrongMint.toBase58() !== REAL_RISE_MINT.toBase58();
const needInitVault   = !vaultInfo || needCloseVault;

if (DRY_RUN) {
  console.log("=== DRY RUN — no transactions sent ===");
  console.log("Re-run with:  node fix_governance.mjs --exec");
  console.log();
  console.log("Steps that will execute:");
  console.log(needCloseVault  ? "  1. close_rise_vault        — burn wrong tokens, close vault, return rent"
                              : "  1. close_rise_vault        — SKIP (vault already correct or absent)");
  console.log(needCloseConfig ? "  2. close_governance_config — close old config, return rent"
                              : "  2. close_governance_config — SKIP (config already has correct mint)");
  console.log(needInitConfig  ? "  3. initialize_governance   — create config with real RISE mint"
                              : "  3. initialize_governance   — SKIP (config already has correct mint)");
  console.log(needInitVault   ? "  4. initialize_rise_vault   — create vault for real RISE mint"
                              : "  4. initialize_rise_vault   — SKIP (vault already correct)");
  console.log();
  console.log("Your RISE tokens at", REAL_RISE_MINT.toBase58(), "are never touched.");
  process.exit(0);
}

// ─── Step 1: close rise_vault ───────────────────────────────────────────────
if (needCloseVault) {
  console.log("Step 1: closing rise_vault (burning wrong-mint tokens)...");
  const tx1 = await program.methods
    .closeRiseVault()
    .accounts({
      authority:     keypair.publicKey,
      config:        configPda,
      riseVault:     vaultPda,
      riseVaultMint: wrongMint,
      tokenProgram:  TOKEN_PROGRAM_ID,
    })
    .rpc();
  console.log("  done:", tx1);
} else {
  console.log("Step 1: rise_vault skipped (already correct or absent).");
}

// ─── Step 2: close governance_config ────────────────────────────────────────
if (needCloseConfig) {
  console.log("Step 2: closing governance_config...");
  const tx2 = await program.methods
    .closeGovernanceConfig()
    .accounts({
      authority: keypair.publicKey,
      config:    configPda,
    })
    .rpc();
  console.log("  done:", tx2);
} else {
  console.log("Step 2: governance_config skipped (already has correct mint).");
}

// ─── Step 3: initialize_governance with correct RISE mint ────────────────────
if (needInitConfig) {
  console.log("Step 3: initializing governance with real RISE mint...");
  const tx3 = await program.methods
    .initializeGovernance(PROPOSAL_THRESHOLD, QUORUM_BPS)
    .accounts({
      authority:     keypair.publicKey,
      config:        configPda,
      riseMint:      REAL_RISE_MINT,
      systemProgram: SystemProgram.programId,
    })
    .rpc();
  console.log("  done:", tx3);
} else {
  console.log("Step 3: governance init skipped (config already correct).");
}

// ─── Step 4: initialize_rise_vault ──────────────────────────────────────────
if (needInitVault) {
  console.log("Step 4: initializing rise_vault with real RISE mint...");
  const tx4 = await program.methods
    .initializeRiseVault()
    .accounts({
      authority:     keypair.publicKey,
      config:        configPda,
      riseVault:     vaultPda,
      riseMint:      REAL_RISE_MINT,
      tokenProgram:  TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .rpc();
  console.log("  done:", tx4);
} else {
  console.log("Step 4: rise_vault init skipped (already correct).");
}

// ─── Verify ─────────────────────────────────────────────────────────────────
console.log();
console.log("Verifying on-chain...");
const [newCfg, newVault] = await Promise.all([
  connection.getAccountInfo(configPda),
  connection.getAccountInfo(vaultPda),
]);

const newMintInConfig = new PublicKey(newCfg.data.slice(40, 72)).toBase58();
const newMintInVault  = new PublicKey(newVault.data.slice(0, 32)).toBase58();
const realMint        = REAL_RISE_MINT.toBase58();

console.log("config.rise_mint :", newMintInConfig);
console.log("vault.mint       :", newMintInVault);
console.log("real RISE mint   :", realMint);

const ok = newMintInConfig === realMint && newMintInVault === realMint;
if (ok) {
  console.log();
  console.log("SUCCESS — governance is correctly configured with the real RISE mint.");
  console.log("Lock RISE will work on the frontend after the next Vercel deploy.");
} else {
  console.log();
  console.log("ERROR — one or both mints still do not match. Please investigate.");
}
