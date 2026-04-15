/**
 * close_proposals.mjs
 *
 * Closes all expired or executed proposals and reclaims rent.
 * Authority-only — uses your local Solana keypair.
 *
 * Usage:
 *   node close_proposals.mjs          # dry-run: lists closeable proposals
 *   node close_proposals.mjs --exec   # sends close transactions
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
const DRY_RUN        = !process.argv.includes("--exec");

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

const config       = await gov.account.governanceConfig.fetch(configPda);
const proposalCount = config.proposalCount.toNumber();
const currentSlot  = await connection.getSlot();

console.log(`Proposal count: ${proposalCount}`);
console.log(`Current slot:   ${currentSlot}\n`);

if (proposalCount === 0) {
  console.log("No proposals to close.");
  process.exit(0);
}

// Fetch all proposals
const proposals = await Promise.all(
  Array.from({ length: proposalCount }, (_, i) => {
    const [pda] = PublicKey.findProgramAddressSync(
      [Buffer.from("proposal"), Buffer.from(new BN(i).toArrayLike(Buffer, "le", 8))],
      GOV_PROGRAM_ID
    );
    return gov.account.proposal.fetch(pda)
      .then((data) => ({ index: i, pda, data }))
      .catch(() => null); // already closed
  })
);

const closeable = proposals.filter((p) => {
  if (!p) return false;
  return p.data.executed || currentSlot > p.data.votingEndSlot.toNumber();
});

const alreadyClosed = proposalCount - proposals.filter(Boolean).length;

console.log(`Already closed: ${alreadyClosed}`);
console.log(`Closeable now:  ${closeable.length}`);
console.log(`Still active:   ${proposals.filter(Boolean).length - closeable.length}\n`);

if (closeable.length === 0) {
  console.log("Nothing to close.");
  process.exit(0);
}

for (const p of closeable) {
  const status = p.data.executed ? "executed" : "expired";
  console.log(`Proposal #${p.index} (${status})${DRY_RUN ? " — would close" : " — closing..."}`);
  if (DRY_RUN) continue;

  try {
    const tx = await gov.methods
      .closeProposal()
      .accounts({
        authority: keypair.publicKey,
        config:    configPda,
        proposal:  p.pda,
      })
      .rpc();
    console.log(`  ✓ tx: ${tx}`);
  } catch (err) {
    console.error(`  ✗ failed: ${err.message}`);
  }
}

if (DRY_RUN) {
  console.log("\nDry run — pass --exec to close them.");
}
