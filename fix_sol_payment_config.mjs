/**
 * fix_sol_payment_config.mjs
 *
 * Corrects the SOL payment config's Pyth price feed on devnet.
 * It was accidentally initialized with the USDC feed address instead of SOL/USD.
 *
 * Usage:
 *   node fix_sol_payment_config.mjs
 */

import { Connection, PublicKey, Keypair, SystemProgram } from "@solana/web3.js";
import pkg from "@coral-xyz/anchor";
const { AnchorProvider, Program, Wallet } = pkg;
import { execSync } from "child_process";
import { readFileSync } from "fs";
import { homedir } from "os";
import { join } from "path";
import { createRequire } from "module";

const require = createRequire(import.meta.url);

const RPC            = "https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6";
const CDP_PROGRAM_ID = new PublicKey("3snPJTuZP9XHNciH7Q5KZzsvk2doxpuoYqWXf8JofEPR");

const CORRECT_SOL_FEED = new PublicKey("J83w4HKfqxwcq3BEMMkPFSppX3gqekLyLJBexebFVkix");

const walletPath = execSync("solana config get keypair").toString()
  .match(/Keypair Path: (.+)/)?.[1]?.trim()
  ?? join(homedir(), ".config/solana/id.json");

const secret  = JSON.parse(readFileSync(walletPath, "utf8"));
const keypair = Keypair.fromSecretKey(Uint8Array.from(secret));
console.log("Authority:", keypair.publicKey.toBase58());

const connection = new Connection(RPC, "confirmed");
const wallet     = new Wallet(keypair);
const provider   = new AnchorProvider(connection, wallet, { commitment: "confirmed" });

const idl = require("./target/idl/rise_cdp.json");
const cdp = new Program(idl, provider);

const [cdpConfig] = PublicKey.findProgramAddressSync(
  [Buffer.from("cdp_config")],
  CDP_PROGRAM_ID
);

const [solPaymentConfig] = PublicKey.findProgramAddressSync(
  [Buffer.from("payment_config"), SystemProgram.programId.toBuffer()],
  CDP_PROGRAM_ID
);

const before = await cdp.account.paymentConfig.fetch(solPaymentConfig);
console.log("Current pythPriceFeed:", before.pythPriceFeed.toBase58());

if (before.pythPriceFeed.toBase58() === CORRECT_SOL_FEED.toBase58()) {
  console.log("Already correct — nothing to do.");
  process.exit(0);
}

console.log("Updating to correct SOL/USD feed:", CORRECT_SOL_FEED.toBase58());

const tx = await cdp.methods
  .updatePaymentConfig(CORRECT_SOL_FEED, null)
  .accounts({
    authority:     keypair.publicKey,
    cdpConfig,
    paymentConfig: solPaymentConfig,
  })
  .rpc({ commitment: "confirmed" });

console.log("tx:", tx);

const after = await cdp.account.paymentConfig.fetch(solPaymentConfig, "confirmed");
console.log("Updated pythPriceFeed:", after.pythPriceFeed.toBase58());
console.log("Done.");
