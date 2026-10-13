// Balance readers for every chain the wallet shows, called from the Yew page.
// Each reads a public endpoint straight from the browser; nothing secret is
// involved — an address in, a balance in base units (a decimal string) out.
// Midnight has no balance query, only a history subscription, and eCash's
// Chronik answers in protobuf, so both are spelled out here. `io` carries
// fetch and WebSocket so the tests can stand in for the network.

const NIGHT = '0'.repeat(64); // Midnight's native token type: 32 zero bytes.

const json = async (io, url, body) => {
  const response = await io.fetch(url, body === undefined ? {} : { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  if (!response.ok) {
    // The backend relay explains itself; a node gets named by its host.
    const reason = await response.json().then(v => v?.error).catch(() => null);
    throw new Error(reason || `${new URL(url, 'http://local').host} answered ${response.status}.`);
  }
  return response.json();
};
const rpc = async (io, url, method, params) => {
  const reply = await json(io, url, { jsonrpc: '2.0', id: 1, method, params });
  if (reply.error) throw new Error(reply.error.message || 'The network refused the request.');
  return reply.result;
};
/** A non-negative integer string, or a refusal: a balance is never guessed. */
const units = value => {
  const text = String(value ?? '');
  if (!/^\d+$/.test(text)) throw new Error('The network reported a balance that is not a number.');
  return BigInt(text);
};

// Each cluster's genesis hash: the Solana counterpart of an EVM chain id.
const GENESIS = {
  'solana-mainnet': '5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d',
  'solana-devnet': 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG',
  'solana-testnet': '4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY',
};
export async function solana(io, net, coin, address) {
  if (GENESIS[net.key] && (await rpc(io, net.endpoint, 'getGenesisHash', [])) !== GENESIS[net.key]) {
    throw new Error(`${net.endpoint} answered as a different cluster. Refusing to continue.`);
  }
  if (coin.standard === 'native') return String(units((await rpc(io, net.endpoint, 'getBalance', [address])).value));
  // One account, derived in Rust (the associated token account): the public
  // mainnet node refuses to scan an owner's token accounts but reads one.
  if (!coin.account) throw new Error(`No token account for ${coin.symbol}.`);
  const result = await rpc(io, net.endpoint, 'getAccountInfo', [coin.account, { encoding: 'jsonParsed', commitment: 'confirmed' }]);
  if (!result?.value) return '0'; // never received this token
  const info = result.value.data?.parsed?.info;
  if (result.value.data?.program !== 'spl-token' || info?.mint !== coin.id) throw new Error(`${coin.symbol}'s token account is not what it should be. Not shown.`);
  // A token account's owner can be handed to someone else; what sits in one
  // this wallet no longer owns is not its balance.
  if (info.owner !== address) return '0';
  // The mint's decimals, not the table's, decide what a unit is.
  const amount = info.tokenAmount;
  if (!amount || amount.decimals !== coin.decimals) throw new Error(`${coin.symbol} reports ${amount?.decimals} decimals, not ${coin.decimals}. Not shown.`);
  return String(units(amount.amount));
}

// Koios sends no CORS header on its replies, so Cardano is read through the
// backend's relay (/api/wallet/cardano): a public address in, Koios's answer out.
const koios = (io, net, path, address) => json(io, '/api/wallet/cardano', { network: net.key, path, address });
export async function cardano(io, net, coin, address) {
  if (coin.standard === 'native') {
    const [info] = await koios(io, net, 'address_info', address);
    return String(info ? units(info.balance) : 0n); // A fresh address has no row.
  }
  const policy = coin.id.slice(0, 56), name = coin.id.slice(56);
  let total = 0n;
  // Summed, not first match: one address can carry an asset on several rows.
  for (const row of await koios(io, net, 'address_assets', address)) {
    if (row.policy_id === policy && row.asset_name === name) total += units(row.quantity);
  }
  return String(total);
}

/**
 * Replay the address's unshielded history over graphql-transport-ws and sum
 * what is still unspent. The progress marker carries the highest transaction
 * id known for the address; until the replay reaches it, a sum would be a
 * confidently wrong balance, so a timeout is a refusal.
 */
export function midnight(io, net, coin, address, timeoutMs = 20000) {
  const url = net.endpoint.replace(/^https:/, 'wss:').replace(/\/$/, '') + '/ws';
  const query = `subscription($addr: UnshieldedAddress!) { unshieldedTransactions(address: $addr) { __typename
    ... on UnshieldedTransactionsProgress { highestTransactionId }
    ... on UnshieldedTransaction { transaction { id } createdUtxos { owner tokenType value spentAtTransaction { id } } } } }`;
  return new Promise((resolve, reject) => {
    const socket = new io.WebSocket(url, 'graphql-transport-ws');
    const created = [];
    let target = null, seen = 0, done = false;
    const finish = (error, value) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      try { socket.send(JSON.stringify({ id: '1', type: 'complete' })); socket.close(); } catch {}
      error ? reject(error) : resolve(value);
    };
    const timer = setTimeout(() => finish(new Error(`${net.name} did not finish replaying this address in time. Try again.`)), timeoutMs);
    const settle = () => {
      if (target === null || seen < target) return;
      let total = 0n;
      for (const utxo of created) {
        if (utxo.owner === address && !utxo.spentAtTransaction && (utxo.tokenType ?? NIGHT) === NIGHT) total += units(utxo.value);
      }
      finish(null, String(total));
    };
    socket.onopen = () => socket.send(JSON.stringify({ type: 'connection_init' }));
    socket.onerror = () => finish(new Error(`Could not reach ${net.name}.`));
    socket.onclose = () => finish(new Error(`${net.name} closed the connection.`));
    socket.onmessage = event => {
      let message;
      try { message = JSON.parse(event.data); } catch { return; }
      if (message.type === 'connection_ack') socket.send(JSON.stringify({ id: '1', type: 'subscribe', payload: { query, variables: { addr: address } } }));
      else if (message.type === 'error') finish(new Error(`${net.name} refused the request.`));
      else if (message.type === 'next') {
        const item = message.payload?.data?.unshieldedTransactions;
        if (!item) return finish(new Error(`${net.name} refused the request.`));
        if (item.__typename === 'UnshieldedTransactionsProgress') target = Number(item.highestTransactionId ?? 0);
        else {
          seen = Math.max(seen, Number(item.transaction?.id ?? 0));
          created.push(...(item.createdUtxos ?? []));
        }
        try { settle(); } catch (error) { finish(error); }
      }
    };
  });
}

// CashAddr (eCash): base32 with a BCH-code checksum over the prefix.
const CHARSET = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l';
function polymod(values) {
  const G = [0x98f2bc8e61n, 0x79b76d99e2n, 0xf33e5fb3c4n, 0xae2eabe2a8n, 0x1e4f43e470n];
  let c = 1n;
  for (const d of values) {
    const c0 = c >> 35n;
    c = ((c & 0x07ffffffffn) << 5n) ^ BigInt(d);
    for (let i = 0; i < 5; i++) if ((c0 >> BigInt(i)) & 1n) c ^= G[i];
  }
  return c ^ 1n;
}
/** The script type and 20-byte hash a CashAddr names, checksum verified. */
export function cashaddr(address) {
  const [prefix, body] = address.toLowerCase().split(':');
  if (!body) throw new Error('That is not an eCash address.');
  const data = [...body].map(c => CHARSET.indexOf(c));
  if (data.some(d => d < 0) || data.length < 8) throw new Error('That is not an eCash address.');
  if (polymod([...[...prefix].map(c => c.charCodeAt(0) & 31), 0, ...data]) !== 0n) throw new Error('That eCash address has a bad checksum.');
  let acc = 0, bits = 0;
  const bytes = [];
  for (const d of data.slice(0, -8)) {
    acc = (acc << 5) | d; bits += 5;
    while (bits >= 8) { bits -= 8; bytes.push((acc >> bits) & 0xff); }
  }
  const [version, ...hash] = bytes;
  if (hash.length < 20) throw new Error('That is not an eCash address.');
  return { type: version >> 3 === 1 ? 'p2sh' : 'p2pkh', hash: hash.slice(0, 20).map(b => b.toString(16).padStart(2, '0')).join('') };
}
/** Read protobuf varints and length-delimited fields; anything else is skipped. */
function* fields(bytes) {
  let i = 0;
  const varint = () => {
    let value = 0n, shift = 0n;
    for (;;) {
      if (i >= bytes.length) throw new Error('Chronik sent a truncated reply.');
      const b = bytes[i++];
      value |= BigInt(b & 0x7f) << shift;
      if (b < 0x80) return value;
      shift += 7n;
    }
  };
  while (i < bytes.length) {
    const tag = varint(), number = Number(tag >> 3n), wire = Number(tag & 7n);
    if (wire === 0) yield [number, varint()];
    else if (wire === 2) {
      const length = Number(varint());
      if (i + length > bytes.length) throw new Error('Chronik sent a truncated reply.');
      yield [number, bytes.subarray(i, i + length)];
      i += length;
    } else if (wire === 1) i += 8;
    else if (wire === 5) i += 4;
    else throw new Error('Chronik sent a reply this wallet cannot read.');
  }
}
/** Total sats in a ScriptUtxos message: field 2 is each Utxo, its field 5 the sats. */
export function chronikTotal(bytes) {
  let total = 0n;
  for (const [number, utxo] of fields(bytes)) {
    if (number !== 2 || typeof utxo === 'bigint') continue;
    for (const [inner, value] of fields(utxo)) if (inner === 5 && typeof value === 'bigint') total += value;
  }
  return total;
}
export async function ecash(io, net, coin, address) {
  const { type, hash } = cashaddr(address);
  const response = await io.fetch(`${net.endpoint}/script/${type}/${hash}/utxos`, { headers: { accept: 'application/x-protobuf' } });
  if (response.status === 404) return '0'; // Chronik's answer for an empty address.
  if (!response.ok) throw new Error(`${net.name} answered ${response.status}.`);
  return String(chronikTotal(new Uint8Array(await response.arrayBuffer())));
}

const READERS = { solana, cardano, midnight, ecash };
/** The balance of `coin` at `address` on a non-EVM network, in base units. */
export function balance(net, coin, address, io = globalThis) {
  const read = READERS[net.chain];
  if (!read) return Promise.reject(new Error(`No balance reader for ${net.name}.`));
  return read(io, net, coin, address);
}
