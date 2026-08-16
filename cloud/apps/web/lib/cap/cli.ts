import { capImport, capabilities, type CapabilityId } from "../capabilities.ts";

const args = process.argv.slice(2);
if (!args.length || args[0] === "-h" || args[0] === "--help") {
  for (const [id, cap] of Object.entries(capabilities)) {
    console.log(`${id}\t${cap.use}`);
  }
  console.error("usage: ./cloud/scripts/use-capability.sh llm composer project-picker");
  process.exit(0);
}

for (const id of args) {
  if (!(id in capabilities)) {
    console.error(`unknown capability: ${id}`);
    console.error(`ids: ${Object.keys(capabilities).join(", ")}`);
    process.exit(1);
  }
  console.log(capImport(id as CapabilityId));
}
