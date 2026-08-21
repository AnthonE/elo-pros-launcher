// build_artifacts.js — compile EloTrove + its mocks to /tmp/artifacts.json
//
//   cd contracts && npm i solc@0.8.26 && node build_artifacts.js
//
// WHY THIS EXISTS. This box has no foundry (ITEM-CONTRACT.md §3a: the egress
// policy blocks the installer), and a contract that has only ever been read is
// a contract nobody has run. solc ships on npm, which the policy allows, so
// between this and py-evm the real bytecode can be deployed and driven —
// see contracts/test_trove_evm.py.
//
// ⚠ It compiles at foundry.toml's EXACT settings (optimizer on, 200 runs,
// via_ir) so the sizes it prints are the sizes forge would print. It is NOT a
// replacement for `forge build --sizes && forge test`, which stays the gate.
const fs=require('fs'), path=require('path'), solc=require('/tmp/node_modules/solc');
const roots=[['src','EloTrove.sol'],['src','EloItem.sol'],['test','TroveMocks.sol']];
const OUT=process.env.OUT||'/tmp/artifacts.json';
const sources={};
function add(dir,f){ const key=(dir==='src'?'':'test/')+f; if(sources[key])return;
  const c=fs.readFileSync(path.join(dir,f),'utf8'); sources[key]={content:c};
  for(const m of c.matchAll(/import\s*\{[^}]*\}\s*from\s*"\.\/([^"]+)"/g)) add('src',m[1]); }
for(const [d,f] of roots) add(d,f);
const input={language:'Solidity',sources,settings:{optimizer:{enabled:true,runs:200},viaIR:true,
  outputSelection:{'*':{'*':['evm.bytecode.object','evm.deployedBytecode.object','abi']}}}};
const out=JSON.parse(solc.compile(JSON.stringify(input)));
let bad=0; for(const e of out.errors||[]){ if(e.severity==='error'){bad++;console.error(e.formattedMessage);} }
if(bad) process.exit(1);
const art={};
for(const f in out.contracts) for(const n in out.contracts[f])
  art[n]={abi:out.contracts[f][n].abi, bin:out.contracts[f][n].evm.bytecode.object,
          runtime:(out.contracts[f][n].evm.deployedBytecode.object||'').length/2};
fs.writeFileSync(OUT, JSON.stringify(art));
for(const n of Object.keys(art)) if(art[n].runtime>0)
  console.log(`${n.padEnd(20)} ${String(art[n].runtime).padStart(7)} B runtime   margin ${24576-art[n].runtime}`);
console.log('->', OUT);
