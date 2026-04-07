import * as anchor from "@coral-xyz/anchor";
import { AnchorProvider, Wallet } from "@coral-xyz/anchor";
import { Connection, Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import * as fs from "fs";

const RPC = "https://devnet.helius-rpc.com/?api-key=787be2ec-9299-40c2-af00-e559a4715fa1";
const STAKING_PROGRAM_ID = new PublicKey("BnQc6jJMT6mt3mvWuQFAd9vf2T2wWkAYD2uGjCXud6Lo");
// eslint-disable-next-line @typescript-eslint/no-unused-vars
const KEYPAIR_PATH = process.env.ANCHOR_WALLET ?? `${process.env.HOME}/.config/solana/id.json`;

async function main() {
  const connection = new Connection(RPC, "confirmed");
  const kp = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(fs.readFileSync(KEYPAIR_PATH, "utf8")))
  );
  const wallet = new Wallet(kp);
  const provider = new AnchorProvider(connection, wallet, { commitment: "confirmed" });
  anchor.setProvider(provider);

  const idl = JSON.parse(
    fs.readFileSync(`${__dirname}/../target/idl/rise_staking.json`, "utf8")
  );
  const program = new anchor.Program(idl, provider) as any;

  const [poolPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("global_pool")],
    STAKING_PROGRAM_ID
  );

  console.log("Calling migrate_global_pool...");
  const tx = await program.methods
    .migrateGlobalPool()
    .accounts({
      authority: kp.publicKey,
      pool: poolPda,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  console.log("Success:", tx);
}

main().catch((err) => { console.error(err); process.exit(1); });
