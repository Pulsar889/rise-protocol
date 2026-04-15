import { Connection, PublicKey, Keypair } from "@solana/web3.js";
import pkg from "@coral-xyz/anchor";
const { AnchorProvider, Program, Wallet } = pkg;
import { execSync } from "child_process";
import { readFileSync } from "fs";
import { homedir } from "os";
import { join } from "path";
import { createRequire } from "module";

const conn = new Connection("https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6", "confirmed");
const GOV  = new PublicKey("CtMKhgY5xKiwLB5jmQ44PRF9QsUqXqSbiyVbFsidskHz");

const walletPath = execSync("solana config get keypair").toString()
  .match(/Keypair Path: (.+)/)?.[1]?.trim()
  ?? join(homedir(), ".config/solana/id.json");
const keypair = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(walletPath, "utf8"))));
const wallet  = new Wallet(keypair);
const provider = new AnchorProvider(conn, wallet, { commitment: "confirmed" });

const require = createRequire(import.meta.url);
const idl = require("./target/idl/rise_governance.json");
const program = new Program(idl, provider);

const owner = keypair.publicKey;
console.log("Wallet:", owner.toBase58());

const currentSlot = await conn.getSlot();
console.log("Current slot:", currentSlot);
console.log();

// Fetch all VeLocks for this wallet
const locks = await program.account.veLock.all([
  { memcmp: { offset: 8, bytes: owner.toBase58() } }
]);

if (locks.length === 0) {
  console.log("No VeLocks found.");
} else {
  for (const { publicKey, account } of locks) {
    const slotsLeft = account.lockEndSlot.toNumber() - currentSlot;
    console.log("VeLock PDA:      ", publicKey.toBase58());
    console.log("nonce:           ", account.nonce);
    console.log("lock_number:     ", account.lockNumber.toString());
    console.log("rise_locked raw: ", account.riseLocked.toString());
    console.log("rise_locked RISE:", account.riseLocked.toNumber() / 1e6, "(at 6 decimals)");
    console.log("verise_amount:   ", account.veriseAmount.toNumber() / 1e6);
    console.log("lock_start_slot: ", account.lockStartSlot.toString());
    console.log("lock_end_slot:   ", account.lockEndSlot.toString());
    console.log("slots remaining: ", slotsLeft);
    console.log("nft_mint:        ", account.nftMint.toBase58());
    console.log();
  }
}
