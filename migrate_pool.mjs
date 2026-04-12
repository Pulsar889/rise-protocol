import { AnchorProvider, Program, Wallet } from "@coral-xyz/anchor";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { readFileSync } from "fs";
import { homedir } from "os";
import path from "path";

const RPC = "https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6";
const STAKING_PROGRAM_ID = new PublicKey("BnQc6jJMT6mt3mvWuQFAd9vf2T2wWkAYD2uGjCXud6Lo");

// Load wallet
const keyPath = path.join(homedir(), ".config/solana/id.json");
const secret = JSON.parse(readFileSync(keyPath, "utf8"));
const keypair = Keypair.fromSecretKey(Uint8Array.from(secret));

// Load IDL
const idlPath = new URL("./target/idl/rise_staking.json", import.meta.url).pathname;
const idl = JSON.parse(readFileSync(idlPath, "utf8"));

const connection = new Connection(RPC, "confirmed");
const wallet = new Wallet(keypair);
const provider = new AnchorProvider(connection, wallet, { commitment: "confirmed" });

const program = new Program(idl, provider);

const [pool] = PublicKey.findProgramAddressSync([Buffer.from("global_pool")], STAKING_PROGRAM_ID);

console.log("Authority:", keypair.publicKey.toBase58());
console.log("Pool PDA:", pool.toBase58());
console.log("Calling migrate_global_pool...");

try {
  const tx = await program.methods
    .migrateGlobalPool()
    .accounts({
      authority: keypair.publicKey,
      pool,
      systemProgram: new PublicKey("11111111111111111111111111111111"),
    })
    .rpc();

  console.log("Migration successful! Tx:", tx);
} catch (err) {
  console.error("Migration failed:", err.message);
  if (err.logs) console.error("Logs:", err.logs);
  process.exit(1);
}
