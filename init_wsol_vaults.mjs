/**
 * init_wsol_vaults.mjs
 *
 * One-time deploy step: creates cdp_wsol_vault and cdp_wsol_buyback_vault.
 * Must be called once after initialize_cdp_config and before any repay_debt calls.
 *
 * Run from rise-protocol/:  node init_wsol_vaults.mjs
 */

import pkg from "@coral-xyz/anchor";
const { AnchorProvider, Program, Wallet } = pkg;
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { readFileSync } from "fs";
import { homedir } from "os";
import path from "path";

const RPC         = "https://devnet.helius-rpc.com/?api-key=787be2ec-9299-40c2-af00-e559a4715fa1";
const CDP_PROG_ID = new PublicKey("3snPJTuZP9XHNciH7Q5KZzsvk2doxpuoYqWXf8JofEPR");
const WSOL_MINT   = new PublicKey("So11111111111111111111111111111111111111112");

const keyPath = path.join(homedir(), ".config/solana/id.json");
const keypair = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(keyPath, "utf8"))));

const idl        = JSON.parse(readFileSync(new URL("./target/idl/rise_cdp.json", import.meta.url).pathname, "utf8"));
const connection = new Connection(RPC, "confirmed");
const provider   = new AnchorProvider(connection, new Wallet(keypair), { commitment: "confirmed" });
const program    = new Program(idl, provider);

const [cdpConfig]          = PublicKey.findProgramAddressSync([Buffer.from("cdp_config")],              CDP_PROG_ID);
const [cdpWsolVault]       = PublicKey.findProgramAddressSync([Buffer.from("cdp_wsol_vault")],          CDP_PROG_ID);
const [cdpWsolBuybackVault] = PublicKey.findProgramAddressSync([Buffer.from("cdp_wsol_buyback_vault")], CDP_PROG_ID);

console.log("Authority:              ", keypair.publicKey.toBase58());
console.log("cdp_config:             ", cdpConfig.toBase58());
console.log("cdp_wsol_vault:         ", cdpWsolVault.toBase58());
console.log("cdp_wsol_buyback_vault: ", cdpWsolBuybackVault.toBase58());

// Check if already initialized
const vaultInfo = await connection.getAccountInfo(cdpWsolVault);
if (vaultInfo) {
  console.log("\ncdp_wsol_vault already exists — nothing to do.");
  process.exit(0);
}

console.log("\nInitializing WSOL vaults...");
try {
  const tx = await program.methods
    .initializeWsolVaults()
    .accounts({
      authority:           keypair.publicKey,
      cdpConfig,
      wsolMint:            WSOL_MINT,
      cdpWsolVault,
      cdpWsolBuybackVault,
      tokenProgram:        TOKEN_PROGRAM_ID,
      systemProgram:       new PublicKey("11111111111111111111111111111111"),
    })
    .rpc();
  console.log("Done. Tx:", tx);
} catch (err) {
  console.error("Failed:", err.message);
  if (err.logs) console.error(err.logs);
  process.exit(1);
}
