// app/lib/rpc.ts

// ตั้งค่า URL ให้ชี้ไปที่ RPC Server ของ Kanari (ค่าเริ่มต้น 127.0.0.1 พอร์ต 19001)
const DEFAULT_RPC_URL = "http://192.168.1.101:19001";
const RPC_URL = process.env.NEXT_PUBLIC_RPC_URL || DEFAULT_RPC_URL;

export const RPC_METHODS = {
  GET_ACCOUNT: "kanari_getAccount",
  GET_TOKEN_BALANCE: "kanari_getTokenBalance",
  LIST_TOKENS: "kanari_listTokens",
  GET_ALL_BALANCES: "kanari_getAllBalances",
  GET_BLOCK: "kanari_getBlock",
  GET_FULL_BLOCK: "kanari_getFullBlock",
  GET_TRANSACTION: "kanari_getTransaction",
  GET_ALL_TRANSACTIONS: "kanari_getAllTransactions",
  GET_BLOCK_HEIGHT: "kanari_getBlockHeight",
  GET_STATS: "kanari_getStats",
  SUBMIT_TRANSACTION: "kanari_submitTransaction",
  HEALTH: "kanari_health",
  GET_NETWORK_STATUS: "kanari_getNetworkStatus",
  PUBLISH_MODULE: "kanari_publishModule",
  GET_MODULE: "kanari_getModule",
  LIST_MODULES: "kanari_listModules",
  VERIFY_MODULE: "kanari_verifyModule",
  CALL_FUNCTION: "kanari_callFunction",
  VIEW_FUNCTION: "kanari_viewFunction",
  GET_OBJECT: "kanari_getObject",
  GET_OWNED_OBJECTS: "kanari_getOwnedObjects",
  GET_OWNED_NFTS: "kanari_getOwnedNfts",
  LIST_COLLECTIONS: "kanari_listCollections",
  GET_NFTS_BY_COLLECTION: "kanari_getNftsByCollection",
} as const;

export type RpcEndpoint = {
  name: string;
  url: string;
};

export type NetworkAuthorityStatus = {
  authority_id: string;
  local: boolean;
};

export type NetworkStatus = {
  local_authority_id: string;
  authority_count: number;
  authorities: NetworkAuthorityStatus[];
};

export type NodeHealth = {
  endpoint: RpcEndpoint;
  online: boolean;
  status: string;
  height: number | null;
  stateRoot: string | null;
  totalTransactions: number | null;
  totalAccounts: number | null;
  pendingTransactions: number | null;
  latencyMs: number | null;
  error?: string;
};

export type RpcBytes = number[];

export type SignedTransactionPayload = {
  sender: string;
  recipient?: string | null;
  amount?: number | null;
  gas_limit: number;
  gas_price: number;
  sequence_number: number;
  signature?: RpcBytes | null;
};

export type PublishModulePayload = {
  sender: string;
  module_bytes: RpcBytes;
  module_name: string;
  gas_limit: number;
  gas_price: number;
  sequence_number: number;
  signature?: RpcBytes | null;
  execute_immediate?: boolean;
};

export type FunctionCallPayload = {
  package: string;
  module: string;
  function: string;
  type_args?: string[];
  args?: RpcBytes[];
};

export type CallFunctionPayload = FunctionCallPayload & {
  sender: string;
  gas_limit: number;
  gas_price: number;
  sequence_number: number;
  signature?: RpcBytes | null;
  execute_immediate?: boolean;
};

export type GetOwnedObjectsOptions = {
  object_type?: string | null;
};

function parseRpcEndpoints(): RpcEndpoint[] {
  const configured = process.env.NEXT_PUBLIC_RPC_ENDPOINTS;
  const endpoints = configured
    ? configured
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean)
      .map((item, index) => {
        const separatorIndex = item.indexOf("=");
        if (separatorIndex > 0) {
          return {
            name: item.slice(0, separatorIndex).trim(),
            url: item.slice(separatorIndex + 1).trim(),
          };
        }
        return { name: `Node ${index + 1}`, url: item };
      })
    : [{ name: "Node 1", url: RPC_URL }];

  return endpoints.filter((endpoint) => endpoint.url.length > 0);
}

export const RPC_ENDPOINTS = parseRpcEndpoints();

function replacePort(url: string, port: number): string {
  try {
    const parsed = new URL(url);
    parsed.port = String(port);
    return parsed.toString().replace(/\/$/, "");
  } catch {
    return url;
  }
}

export function deriveAuthorityRpcEndpoints(
  baseUrl: string,
  networkStatus: NetworkStatus
): RpcEndpoint[] {
  const basePort = (() => {
    try {
      const parsed = new URL(baseUrl);
      return Number(parsed.port || (parsed.protocol === "https:" ? 443 : 80));
    } catch {
      return 19001;
    }
  })();

  return networkStatus.authorities.map((authority, index) => ({
    name: `Node ${index + 1} (${authority.authority_id})`,
    url: replacePort(baseUrl, basePort + index * 10),
  }));
}

// Explorer pages currently consume mixed JSON-RPC shapes directly.
export async function callRpc(
  method: string,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  params: any = [],
  rpcUrl: string = RPC_URL
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
): Promise<any> {
  const body = {
    jsonrpc: "2.0",
    method,
    params,
    id: Date.now(),
  };

  try {
    const res = await fetch(rpcUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });

    if (!res.ok) {
      throw new Error(`RPC HTTP Error: ${res.status}`);
    }

    const data = await res.json();

    // 🚨 FIX 1: ดักจับ Error ให้พิมพ์ชื่อ Method ออกมาด้วย จะได้รู้ว่าตัวไหนพัง
    if (data.error) {
      console.warn(`[RPC Alert] Method: ${method} failed.`, data.error.message);
      throw new Error(`[${method}] ${data.error.message || "Unknown RPC Error"}`);
    }

    return data.result ?? data;
  } catch (err) {
    console.error(`[RPC Error] ${method}:`, err);
    throw err;
  }
}
export async function getTokenBalance(address: string, token_type: string) {
  return callRpc(RPC_METHODS.GET_TOKEN_BALANCE, { address, token_type });
}

export async function getAllBalances(address: string) {
  const resp = await callRpc(RPC_METHODS.GET_ALL_BALANCES, { address });
  return resp?.balances ?? resp;
}

export async function getAccount(address: string) {
  return callRpc(RPC_METHODS.GET_ACCOUNT, address);
}

export async function getTokens() {
  return callRpc(RPC_METHODS.LIST_TOKENS, []);
}

export async function getStats(rpcUrl?: string) {
  return callRpc(RPC_METHODS.GET_STATS, [], rpcUrl);
}

export async function getHealth(rpcUrl?: string) {
  return callRpc(RPC_METHODS.HEALTH, [], rpcUrl);
}

export async function getNetworkStatus(rpcUrl?: string): Promise<NetworkStatus> {
  return callRpc(RPC_METHODS.GET_NETWORK_STATUS, [], rpcUrl);
}

export async function getNodeHealth(endpoint: RpcEndpoint): Promise<NodeHealth> {
  const startedAt = performance.now();
  try {
    const [stats, health] = await Promise.all([
      getStats(endpoint.url),
      getHealth(endpoint.url).catch(() => null),
    ]);
    return {
      endpoint,
      online: true,
      status: String(readField(health, "status") ?? "ok"),
      height: Number(readField(stats, "height") ?? 0),
      stateRoot: String(readField(stats, "state_root") ?? readField(stats, "stateRoot") ?? "") || null,
      totalTransactions: Number(readField(stats, "total_transactions") ?? 0),
      totalAccounts: Number(readField(stats, "total_accounts") ?? 0),
      pendingTransactions: Number(readField(stats, "pending_transactions") ?? 0),
      latencyMs: Math.round(performance.now() - startedAt),
    };
  } catch (err) {
    return {
      endpoint,
      online: false,
      status: "offline",
      height: null,
      stateRoot: null,
      totalTransactions: null,
      totalAccounts: null,
      pendingTransactions: null,
      latencyMs: null,
      error: err instanceof Error ? err.message : "RPC unavailable",
    };
  }
}

// 🚨 FIX 2: ดักจับ Error ของ getBlockHeight แบบเงียบๆ ป้องกันหน้า Home พังหากฝั่ง Rust ยังไม่ได้ทำ API นี้ไว้
export async function getBlockHeight() {
  try {
    return await callRpc(RPC_METHODS.GET_BLOCK_HEIGHT, []);
  } catch {
    console.warn("getBlockHeight not available yet, returning null.");
    return null;
  }
}

export async function getBlock(height: number, rpcUrl?: string) {
  return callRpc(RPC_METHODS.GET_BLOCK, height, rpcUrl);
}

export async function getFullBlock(height: number, rpcUrl?: string) {
  return callRpc(RPC_METHODS.GET_FULL_BLOCK, height, rpcUrl);
}

function readField(value: unknown, field: string): unknown {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return undefined;
  return (value as Record<string, unknown>)[field];
}

function dedupeTransactions<T>(transactions: T[]): T[] {
  const seen = new Set<string>();
  return transactions.filter((transaction) => {
    const hash = readField(transaction, "hash");
    if (typeof hash !== "string") return true;

    const normalized = hash.toLowerCase();
    if (seen.has(normalized)) return false;
    seen.add(normalized);
    return true;
  });
}

// ดึงประวัติธุรกรรมทั้งหมด (รองรับ Limit และการกรองด้วย Account)
export async function getAllTransactions(limit: number = 50, account?: string) {
  const params: { limit: number; account?: string } = { limit };
  if (account) params.account = account;
  const response = await callRpc(RPC_METHODS.GET_ALL_TRANSACTIONS, params);
  if (Array.isArray(response)) return dedupeTransactions(response);

  const result = readField(response, "result");
  if (Array.isArray(result)) return dedupeTransactions(result);

  const transactions = readField(response, "transactions");
  if (Array.isArray(transactions)) return dedupeTransactions(transactions);

  const data = readField(response, "data");
  if (Array.isArray(data)) return dedupeTransactions(data);

  return response;
}

// ค้นหาธุรกรรมแบบเจาะจงด้วย Hash
export async function getTransaction(hash: string) {
  return callRpc(RPC_METHODS.GET_TRANSACTION, { hash });
}

export async function submitTransaction(transaction: SignedTransactionPayload) {
  return callRpc(RPC_METHODS.SUBMIT_TRANSACTION, transaction);
}

export async function publishModule(payload: PublishModulePayload) {
  return callRpc(RPC_METHODS.PUBLISH_MODULE, payload);
}

export async function getModule(address: string, name: string) {
  return callRpc(RPC_METHODS.GET_MODULE, { address, name });
}

export async function listModules() {
  return callRpc(RPC_METHODS.LIST_MODULES, []);
}

export async function verifyModule(module_bytes: RpcBytes) {
  return callRpc(RPC_METHODS.VERIFY_MODULE, { module_bytes });
}

export async function callFunction(payload: CallFunctionPayload) {
  return callRpc(RPC_METHODS.CALL_FUNCTION, {
    ...payload,
    args: payload.args ?? [],
    type_args: payload.type_args ?? [],
  });
}

export async function viewFunction(payload: FunctionCallPayload) {
  return callRpc(RPC_METHODS.VIEW_FUNCTION, [
    {
      ...payload,
      args: payload.args ?? [],
      type_args: payload.type_args ?? [],
    },
  ]);
}

export async function getObject(object_id: string) {
  return callRpc(RPC_METHODS.GET_OBJECT, { object_id });
}

export async function getOwnedObjects(owner: string, options: GetOwnedObjectsOptions = {}) {
  const response = await callRpc(RPC_METHODS.GET_OWNED_OBJECTS, {
    owner,
    object_type: options.object_type ?? null,
  });
  return response?.objects ?? response;
}

export async function getOwnedNfts(address: string) {
  return callRpc(RPC_METHODS.GET_OWNED_NFTS, address);
}

export async function getCollections() {
  return callRpc(RPC_METHODS.LIST_COLLECTIONS, []);
}

export async function getNftsByCollection(collectionId: string) {
  return callRpc(RPC_METHODS.GET_NFTS_BY_COLLECTION, collectionId);
}
