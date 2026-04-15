/**
 * update_proposal_threshold.mjs
 *
 * Calls update_governance_config to set proposal_threshold to 10,000 RISE.
 *
 * Usage:
 *   node update_proposal_threshold.mjs          # dry-run
 *   node update_proposal_threshold.mjs --exec   # send tx
 */

import { Connection, PublicKey, Keypair } from "@solana/web3.js";
import pkg from "@coral-xyz/anchor";
const { AnchorProvider, Program, Wallet, BN } = pkg;
import { execSync } from "child_process";
import { readFileSync } from "fs";
import { homedir } from "os";
import { join } from "path";
import { createRequire } from "module";

const require = createRequire(import.meta.url);


const RPC            = "https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6";
const GOV_PROGRAM_ID = new PublicKey("CtMKhgY5xKiwLB5jmQ44PRF9QsUqXqSbiyVbFsidskHz");
const NEW_THRESHOLD  = new BN("10000000000"); // 10,000 RISE (6 decimals)

const DRY_RUN = !process.argv.includes("--exec");

const walletPath = execSync("solana config get keypair").toString()
  .match(/Keypair Path: (.+)/)?.[1]?.trim()
  ?? join(homedir(), ".config/solana/id.json");

const secret  = JSON.parse(readFileSync(walletPath, "utf8"));
const keypair = Keypair.fromSecretKey(Uint8Array.from(secret));
console.log("Authority:", keypair.publicKey.toBase58());

const connection = new Connection(RPC, "confirmed");
const wallet     = new Wallet(keypair);
const provider   = new AnchorProvider(connection, wallet, { commitment: "confirmed" });

const idl = require("./target/idl/rise_governance.json");
const gov = new Program(idl, provider);

const [configPda] = PublicKey.findProgramAddressSync(
  [Buffer.from("governance_config")],
  GOV_PROGRAM_ID
);

const config = await gov.account.governanceConfig.fetch(configPda);
const currentThreshold = config.proposalThreshold.toString();
console.log(`Current proposal_threshold: ${currentThreshold} (${Number(currentThreshold) / 1_000_000} RISE)`);
console.log(`New     proposal_threshold: ${NEW_THRESHOLD.toString()} (${NEW_THRESHOLD.toNumber() / 1_000_000} RISE)`);

if (DRY_RUN) {
  console.log("\nDry run — pass --exec to send the transaction.");
  process.exit(0);
}

const tx = await gov.methods
  .updateGovernanceConfig(
    NEW_THRESHOLD, // proposal_threshold
    null,          // quorum_bps — unchanged
    null,          // voting_period_slots — unchanged
    null,          // timelock_slots — unchanged
  )
  .accounts({
    authority: keypair.publicKey,
    config:    configPda,
  })
  .rpc();

console.log("\nTransaction:", tx);
console.log("proposal_threshold updated to 10,000 RISE.");
