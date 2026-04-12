import { Connection, PublicKey } from "@solana/web3.js";
import { execSync } from "child_process";

const conn = new Connection("https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6", "confirmed");

const wallet = execSync("solana address").toString().trim();
console.log("Wallet:", wallet);

const TOKEN_PROGRAM = new PublicKey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

const accounts = await conn.getParsedTokenAccountsByOwner(
  new PublicKey(wallet),
  { programId: TOKEN_PROGRAM }
);

for (const { account } of accounts.value) {
  const info = account.data.parsed.info;
  console.log("mint:", info.mint, "| balance:", info.tokenAmount.uiAmount);
}
