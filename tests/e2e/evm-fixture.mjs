// Test-only helpers for the wallet harness: keccak-256 (to answer
// eth_sendRawTransaction with the real hash), a minimal RLP decoder, and a
// fake Cronos JSON-RPC node for Playwright to route the browser to.
const RC = [
  0x0000000000000001n, 0x0000000000008082n, 0x800000000000808an, 0x8000000080008000n, 0x000000000000808bn, 0x0000000080000001n,
  0x8000000080008081n, 0x8000000000008009n, 0x000000000000008an, 0x0000000000000088n, 0x0000000080008009n, 0x000000008000000an,
  0x000000008000808bn, 0x800000000000008bn, 0x8000000000008089n, 0x8000000000008003n, 0x8000000000008002n, 0x8000000000000080n,
  0x000000000000800an, 0x800000008000000an, 0x8000000080008081n, 0x8000000000008080n, 0x0000000080000001n, 0x8000000080008008n,
];
const ROT = [0, 1, 62, 28, 27, 36, 44, 6, 55, 20, 3, 10, 43, 25, 39, 41, 45, 15, 21, 8, 18, 2, 61, 56, 14];
const M = (1n << 64n) - 1n;
const rotl = (x, n) => n === 0 ? x : ((x << BigInt(n)) | (x >> BigInt(64 - n))) & M;
function permute(s) {
  for (let round = 0; round < 24; round++) {
    const c = [0, 1, 2, 3, 4].map(x => s[x] ^ s[x + 5] ^ s[x + 10] ^ s[x + 15] ^ s[x + 20]);
    for (let x = 0; x < 5; x++) { const d = c[(x + 4) % 5] ^ rotl(c[(x + 1) % 5], 1); for (let y = 0; y < 25; y += 5) s[x + y] ^= d; }
    const b = new Array(25);
    for (let x = 0; x < 5; x++) for (let y = 0; y < 5; y++) b[y + 5 * ((2 * x + 3 * y) % 5)] = rotl(s[x + 5 * y], ROT[x + 5 * y]);
    for (let x = 0; x < 5; x++) for (let y = 0; y < 5; y++) s[x + 5 * y] = b[x + 5 * y] ^ ((~b[(x + 1) % 5 + 5 * y] & M) & b[(x + 2) % 5 + 5 * y]);
    s[0] ^= RC[round];
  }
}
export function keccak256(bytes) {
  const rate = 136, s = new Array(25).fill(0n);
  const padded = new Uint8Array(Math.ceil((bytes.length + 1) / rate) * rate);
  padded.set(bytes); padded[bytes.length] ^= 0x01; padded[padded.length - 1] ^= 0x80;
  for (let off = 0; off < padded.length; off += rate) {
    for (let i = 0; i < rate / 8; i++) {
      let lane = 0n;
      for (let j = 7; j >= 0; j--) lane = (lane << 8n) | BigInt(padded[off + i * 8 + j]);
      s[i] ^= lane;
    }
    permute(s);
  }
  const out = new Uint8Array(32);
  for (let i = 0; i < 4; i++) for (let j = 0; j < 8; j++) out[i * 8 + j] = Number((s[i] >> BigInt(8 * j)) & 0xffn);
  return out;
}
export const hex = bytes => '0x' + Buffer.from(bytes).toString('hex');
export const unhex = text => Uint8Array.from(Buffer.from(text.replace(/^0x/, ''), 'hex'));
/** Decode one RLP item: Uint8Array for strings, arrays for lists. */
export function rlpDecode(bytes) {
  const item = (i) => {
    const p = bytes[i];
    const len = (n, at) => { let v = 0; for (let k = 0; k < n; k++) v = v * 256 + bytes[at + k]; return v; };
    if (p < 0x80) return [bytes.slice(i, i + 1), i + 1];
    if (p <= 0xb7) return [bytes.slice(i + 1, i + 1 + p - 0x80), i + 1 + p - 0x80];
    if (p <= 0xbf) { const n = p - 0xb7, l = len(n, i + 1); return [bytes.slice(i + 1 + n, i + 1 + n + l), i + 1 + n + l]; }
    const [n, l] = p <= 0xf7 ? [0, p - 0xc0] : [p - 0xf7, len(p - 0xf7, i + 1)];
    const end = i + 1 + n + l, out = [];
    for (let at = i + 1 + n; at < end;) { const [v, next] = item(at); out.push(v); at = next; }
    return [out, end];
  };
  return item(0)[0];
}
export const big = bytes => bytes.length ? BigInt(hex(bytes)) : 0n;
const word = n => '0x' + BigInt(n).toString(16).padStart(64, '0');
export const RPC = /^https:\/\/evm(-t3)?\.cronos\.org\/?$/;
export const USDC = { mainnet: '0xc21223249ca28397b4b6541dffaecc539bff0c59', testnet: '0xc01efaaf7c5c61bebfaeb358e1161b537b8bc0e0' };
/** ABI-encode a `string` return: offset, length, padded bytes. */
export function abiString(text) {
  const bytes = Buffer.from(text, 'utf8');
  const padded = Buffer.alloc(Math.ceil(bytes.length / 32) * 32 || 32);
  bytes.copy(padded);
  return word(32) + word(bytes.length).slice(2) + padded.toString('hex');
}
/**
 * Route both Cronos RPC endpoints to a fake node. `state` is mutable so a test
 * can change balances; every call is recorded in `state.calls`.
 */
export async function fakeCronos(context, state) {
  state.calls = [];
  state.sent = [];
  state.polls = {};
  await context.route(RPC, async route => {
    const url = route.request().url();
    const mainnet = !url.includes('-t3');
    const { id, method, params } = JSON.parse(route.request().postData());
    state.calls.push({ url, method, params });
    const answer = result => route.fulfill({ status: 200, contentType: 'application/json', headers: { 'access-control-allow-origin': '*' }, body: JSON.stringify({ jsonrpc: '2.0', id, result }) });
    switch (method) {
      case 'eth_chainId': return answer(mainnet ? '0x19' : '0x152');
      case 'eth_getBalance': return answer('0x' + state.native.toString(16));
      case 'eth_getCode': {
        const to = params[0].toLowerCase();
        return answer(to === (mainnet ? USDC.mainnet : USDC.testnet) || state.tokens?.[to] ? '0x6080604052' : '0x');
      }
      case 'eth_call': {
        // Answer by contract and selector, like a node: USDC, any token in
        // `state.tokens` (keyed by lowercase address), or nothing at all.
        const to = params[0].to.toLowerCase(), selector = params[0].data.slice(0, 10);
        const usdc = to === (mainnet ? USDC.mainnet : USDC.testnet);
        const token = usdc ? { symbol: mainnet ? 'USDC' : 'devUSDC.e', name: 'USD Coin', decimals: 6, balance: mainnet ? state.mainnetUsdc ?? 0n : state.usdc } : state.tokens?.[to];
        if (!token) return answer('0x');
        if (selector === '0x70a08231') return answer(word(token.balance ?? 0n));
        if (selector === '0x313ce567') return answer(word(token.decimals));
        if (selector === '0x95d89b41') return answer(token.bytes32 ? '0x' + Buffer.from(token.symbol).toString('hex').padEnd(64, '0') : abiString(token.symbol));
        if (selector === '0x06fdde03') return token.name == null ? answer('0x') : answer(abiString(token.name));
        return answer('0x');
      }
      case 'eth_getTransactionCount': return answer('0x' + (state.nonce ?? 7).toString(16));
      case 'eth_gasPrice': return answer('0x' + (state.gasPrice ?? 5_000_000_000_000n).toString(16));
      // A plain CRO payment costs exactly 21,000; a token call is estimated at 50,000.
      case 'eth_estimateGas': return answer(!params[0].data || params[0].data === '0x' ? '0x5208' : '0xc350');
      case 'eth_sendRawTransaction': {
        const raw = params[0];
        state.sent.push({ url, raw });
        return answer(hex(keccak256(unhex(raw))));
      }
      case 'eth_getTransactionReceipt': {
        // `receiptAfter` polls answer null (still pending) before the receipt.
        const seen = (state.polls[params[0]] = (state.polls[params[0]] ?? 0) + 1);
        if (seen <= (state.receiptAfter ?? 0)) return answer(null);
        state.onReceipt?.(params[0]);
        return answer({ status: state.receiptStatus ?? '0x1', transactionHash: params[0] });
      }
      default: return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ jsonrpc: '2.0', id, error: { code: -32601, message: 'method not found' } }) });
    }
  });
}
