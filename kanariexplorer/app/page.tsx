"use client";

import Image from "next/image";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import {
  deriveAuthorityRpcEndpoints,
  getBlock,
  getBlockHeight,
  getFullBlock,
  getNetworkStatus,
  getNodeHealth,
  getTokens,
  RPC_ENDPOINTS,
  type NodeHealth,
  type RpcEndpoint,
} from "./lib/rpc";
import { ArrowIcon } from "./components/SiteChrome";
import { asArray, CopyButton, formatNumber, Panel, readString, SearchForm, StatCard, StatusPill } from "./components/ExplorerUI";

const ROOT_SCAN_DEPTH = 8;
const ROOT_SCAN_ENDPOINT_LIMIT = 8;
const NETWORK_POLL_INTERVAL_MS = 750;
const ROOT_SCAN_INTERVAL_MS = 5000;
const NETWORK_POLL_LABEL = `${NETWORK_POLL_INTERVAL_MS} ms`;

type StabilitySample = {
  height: number;
  online: number;
  pending: number;
  timestamp: number;
};

type StateRootCheck = {
  endpoint: string;
  error?: string;
  height: number | null;
  node: string;
  nodeHeight: number | null;
  online: boolean;
  root: string | null;
};

type DivergenceScan = {
  checks: StateRootCheck[];
  height: number;
};

function shortUrl(url: string) {
  return url.replace(/^https?:\/\//, "").replace(/\/$/, "");
}

function shortRoot(root: string | null) {
  if (!root) return "-";
  return root.length > 22 ? `${root.slice(0, 10)}...${root.slice(-8)}` : root;
}

function formatDuration(ms: number | null) {
  if (ms === null) return "No event";
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)} s`;
}

function maxNullable(values: Array<number | null | undefined>) {
  const numbers = values.filter((value): value is number => typeof value === "number" && Number.isFinite(value));
  return numbers.length ? Math.max(...numbers) : null;
}

function readNumber(source: unknown, ...keys: string[]) {
  for (const key of keys) {
    const raw = readString(source, key, "");
    if (!raw || raw === "-") continue;
    const parsed = Number(raw);
    if (Number.isFinite(parsed)) return parsed;
  }
  return null;
}

function readFirstString(source: unknown, ...keys: string[]) {
  for (const key of keys) {
    const value = readString(source, key, "");
    if (value && value !== "-") return value;
  }
  return "";
}

function isLaggingAt(check: StateRootCheck, height: number | null) {
  return check.online && height !== null && check.nodeHeight !== null && check.nodeHeight < height;
}

function comparableRootChecks(checks: StateRootCheck[], height: number | null) {
  if (height === null) return [];
  return checks.filter((check) => check.height === height && check.root && !isLaggingAt(check, height));
}

function groupRoots(checks: StateRootCheck[]) {
  const groups = new Map<string, StateRootCheck[]>();
  checks.forEach((check) => {
    if (!check.root) return;
    const list = groups.get(check.root) ?? [];
    list.push(check);
    groups.set(check.root, list);
  });
  return [...groups.entries()]
    .map(([root, members]) => ({ members, root }))
    .sort((a, b) => b.members.length - a.members.length);
}

async function readStateRootChecksAtHeight(
  height: number,
  endpoints: RpcEndpoint[],
  nodeByUrl: Map<string, NodeHealth>,
): Promise<StateRootCheck[]> {
  return Promise.all(
    endpoints.map(async (endpoint): Promise<StateRootCheck> => {
      const node = nodeByUrl.get(endpoint.url);
      if (!node?.online) {
        return {
          endpoint: endpoint.url,
          height: null,
          node: endpoint.name,
          nodeHeight: node?.height ?? null,
          online: false,
          root: null,
        };
      }

      const nodeHeight = node.height ?? null;
      if (nodeHeight !== null && nodeHeight < height) {
        return {
          endpoint: endpoint.url,
          error: `lagging at height ${nodeHeight}`,
          height,
          node: endpoint.name,
          nodeHeight,
          online: true,
          root: null,
        };
      }

      try {
        const checkpoint = await getFullBlock(height, endpoint.url).catch(() => getBlock(height, endpoint.url).catch(() => null));
        const returnedHeight = readNumber(checkpoint, "height", "checkpoint_height", "block_height", "number");
        const latestRootAtHeight = nodeHeight === height && node?.stateRoot ? node.stateRoot : null;
        if (returnedHeight !== height) {
          return {
            endpoint: endpoint.url,
            error: latestRootAtHeight
              ? undefined
              : returnedHeight === null
                ? "checkpoint height unavailable"
                : `returned height ${returnedHeight}`,
            height,
            node: endpoint.name,
            nodeHeight,
            online: true,
            root: latestRootAtHeight,
          };
        }

        const root = readFirstString(checkpoint, "state_root", "stateRoot", "root");
        return {
          endpoint: endpoint.url,
          height,
          node: endpoint.name,
          nodeHeight,
          online: true,
          root: root && root !== "-" ? root : null,
        };
      } catch (err) {
        return {
          endpoint: endpoint.url,
          error: err instanceof Error ? err.message : "checkpoint unavailable",
          height,
          node: endpoint.name,
          nodeHeight,
          online: true,
          root: null,
        };
      }
    }),
  );
}

function NetworkGraphic() {
  return (
    <div className="network-graphic" aria-hidden="true">
      <span className="network-glow network-glow--one" />
      <span className="network-glow network-glow--two" />
      <div className="network-rotor">
        <div className="network-orbit network-orbit--one" />
        <div className="network-orbit network-orbit--two" />
        <div className="network-orbit network-orbit--three" />
        <span className="network-node network-node--one"><span>01</span></span>
        <span className="network-node network-node--two"><span>K</span></span>
        <span className="network-node network-node--three"><span>+</span></span>
        <span className="network-node network-node--four"><span>M</span></span>
        <span className="network-spark network-spark--one" />
        <span className="network-spark network-spark--two" />
        <span className="network-spark network-spark--three" />
      </div>
      <div className="network-core">
        <Image src="/kariicon1.png" alt="" width={92} height={92} priority />
      </div>
    </div>
  );
}

function ConsensusStabilityPanel({
  blockHeight,
  lastRecoveryMs,
  nodes,
  pendingTransactions,
  samples,
  stateRootChecks,
}: {
  blockHeight: number | null;
  lastRecoveryMs: number | null;
  nodes: NodeHealth[];
  pendingTransactions: number;
  samples: StabilitySample[];
  stateRootChecks: StateRootCheck[];
}) {
  const onlineNodes = nodes.filter((node) => node.online);
  const maxHeight = Math.max(0, ...nodes.map((node) => node.height ?? 0), blockHeight ?? 0);
  const minOnlineHeight = Math.min(...onlineNodes.map((node) => node.height ?? maxHeight));
  const heightSpread = onlineNodes.length > 0 ? maxHeight - minOnlineHeight : 0;
  const laggingNodes = onlineNodes.filter((node) => (node.height ?? 0) < maxHeight).length;
  const offlineNodes = nodes.length - onlineNodes.length;
  const missedVoteEstimate = laggingNodes + offlineNodes;
  const comparisonHeight = stateRootChecks.find((check) => check.height !== null)?.height ?? null;
  const comparableChecks = comparableRootChecks(stateRootChecks, comparisonHeight);
  const roots = comparableChecks
    .map((check) => check.root)
    .filter((root): root is string => Boolean(root));
  const uniqueRootCount = new Set(roots).size;
  const forkCounter = roots.length > 1 ? Math.max(0, uniqueRootCount - 1) : 0;
  const rootStatus = roots.length < 2 ? "Insufficient" : uniqueRootCount === 1 ? "Match" : "Diverged";
  const partitionState = offlineNodes > 0 || heightSpread > 1 || uniqueRootCount > 1 ? "Watch" : "Clear";
  const firstSample = samples[0];
  const lastSample = samples[samples.length - 1];
  const productionRate =
    firstSample && lastSample && lastSample.timestamp > firstSample.timestamp
      ? ((lastSample.height - firstSample.height) / ((lastSample.timestamp - firstSample.timestamp) / 1000)) * 60
      : null;
  const finalityLabel =
    pendingTransactions === 0 && partitionState === "Clear" && roots.length > 0 ? "1 poll" : pendingTransactions > 0 ? "Pending" : "Watching";

  const metrics = [
    {
      detail: finalityLabel === "1 poll" ? "observed stable in current polling window" : "needs finality timestamp RPC for exact ms",
      label: "Finality Time",
      tone: finalityLabel === "Pending" ? "warn" : "ok",
      value: finalityLabel,
    },
    {
      detail: samples.length > 1 ? "derived from height delta over recent polls" : "waiting for more samples",
      label: "Checkpoint Production Rate",
      tone: productionRate === null ? "idle" : "ok",
      value: productionRate === null ? "Collecting" : `${productionRate.toFixed(2)} / min`,
    },
    {
      detail: lastRecoveryMs === null ? "no offline-to-online transition observed" : "latest node recovery window",
      label: "Node Recovery Time",
      tone: lastRecoveryMs === null ? "idle" : "ok",
      value: formatDuration(lastRecoveryMs),
    },
    {
      detail: "inferred from lagging or offline authorities",
      label: "Missed Vote",
      tone: missedVoteEstimate > 0 ? "warn" : "ok",
      value: formatNumber(missedVoteEstimate),
    },
    {
      detail: roots.length > 1 ? `comparable roots checked at height ${formatNumber(comparisonHeight)}` : "requires at least two exact-height roots",
      label: "Fork Counter",
      tone: forkCounter > 0 ? "down" : roots.length > 1 ? "ok" : "idle",
      value: roots.length > 1 ? formatNumber(forkCounter) : "-",
    },
    {
      detail: `height spread ${formatNumber(heightSpread)}, offline ${formatNumber(offlineNodes)}`,
      label: "Network Partition Detector",
      tone: partitionState === "Clear" ? "ok" : "warn",
      value: partitionState,
    },
    {
      detail:
        rootStatus === "Match"
          ? `${roots.length} nodes share one state root`
          : rootStatus === "Diverged"
            ? `${uniqueRootCount} unique roots observed`
            : "waiting for exact-height checkpoint reads",
      label: "State Root Comparison",
      tone: rootStatus === "Diverged" ? "down" : rootStatus === "Match" ? "ok" : "idle",
      value: rootStatus,
    },
  ];

  return (
    <section className="panel consensus-stability-panel">
      <div className="panel-head">
        <div>
          <h2 className="panel-title">Consensus Stability</h2>
          <p className="panel-subtitle">Observed health signals for Mysticeti checkpoint stability</p>
        </div>
        <StatusPill label={partitionState === "Clear" ? "Stable" : "Watching"} state={partitionState === "Clear" ? "ok" : "warn"} />
      </div>
      <div className="stability-grid">
        {metrics.map((metric) => (
          <div className={`stability-card stability-card--${metric.tone}`} key={metric.label}>
            <p className="tiny-label">{metric.label}</p>
            <strong className="mono">{metric.value}</strong>
            <span>{metric.detail}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

function StateDivergenceAudit({
  divergenceWindow,
  stateRootChecks,
}: {
  divergenceWindow: DivergenceScan[];
  stateRootChecks: StateRootCheck[];
}) {
  const comparisonHeight = stateRootChecks.find((check) => check.height !== null)?.height ?? null;
  const comparableChecks = comparableRootChecks(stateRootChecks, comparisonHeight);
  const groups = groupRoots(comparableChecks);
  const readableChecks = comparableChecks.filter((check) => check.root);
  const onlineChecks = stateRootChecks.filter((check) => check.online && !isLaggingAt(check, comparisonHeight));
  const quorum = onlineChecks.length > 0 ? Math.floor((onlineChecks.length * 2) / 3) + 1 : 0;
  const leader = groups[0];
  const leaderCount = leader?.members.length ?? 0;
  const leadingRoot = leader?.root ?? null;
  const canonicalRoot = leader && leaderCount >= quorum ? leader.root : null;
  const rootMode = canonicalRoot ? "Quorum" : leadingRoot ? "Leading" : "Unknown";
  const currentSplit = groups.length > 1;
  const auditStatus = !currentSplit ? "Aligned" : canonicalRoot ? "Outlier" : "Diverged";
  const auditState = auditStatus === "Aligned" ? "ok" : auditStatus === "Outlier" ? "warn" : "down";
  const divergingNodes =
    leadingRoot === null
      ? []
      : comparableChecks.filter((check) => check.online && check.root && check.root !== leadingRoot);
  const firstDivergence = divergenceWindow.find((scan) => groupRoots(comparableRootChecks(scan.checks, scan.height)).length > 1);
  const rootSummary =
    canonicalRoot ??
    leadingRoot ??
    null;
  const suspectLabels = divergingNodes.map((check) => check.node).join(", ");

  const cards = [
    {
      detail: divergingNodes.length > 0 ? suspectLabels : readableChecks.length > 1 ? "all readable roots agree" : "waiting for multiple readable nodes",
      label: canonicalRoot ? "Root Outliers" : "Diverging Nodes",
      tone: divergingNodes.length > 0 ? (canonicalRoot ? "warn" : "down") : readableChecks.length > 1 ? "ok" : "idle",
      value: divergingNodes.length > 0 ? formatNumber(divergingNodes.length) : readableChecks.length > 1 ? "None" : "Collecting",
    },
    {
      detail: firstDivergence ? `first observed split in last ${ROOT_SCAN_DEPTH} checked checkpoints` : "no split in recent root window",
      label: "Divergence Start Checkpoint",
      tone: firstDivergence ? "down" : "ok",
      value: firstDivergence ? formatNumber(firstDivergence.height) : "None",
    },
    {
      detail:
        rootMode === "Quorum"
          ? `${leaderCount}/${onlineChecks.length} nodes agree at height ${formatNumber(comparisonHeight)}`
          : rootMode === "Leading"
            ? `${leaderCount}/${onlineChecks.length} nodes share the leading root, below quorum`
            : "state root RPC reads unavailable",
      label: "Canonical Root",
      tone: canonicalRoot ? "ok" : leadingRoot ? "warn" : "idle",
      value: rootMode,
    },
    {
      detail: divergingNodes.length > 0 ? `repair or resync: ${suspectLabels}` : "no root outliers detected",
      label: "Repair Hint",
      tone: divergingNodes.length > 0 ? "warn" : "idle",
      value: divergingNodes.length > 0 ? "Resync" : "None",
    },
  ];

  return (
    <section className="panel state-audit-panel">
      <div className="panel-head">
        <div>
          <h2 className="panel-title">State Divergence Audit</h2>
          <p className="panel-subtitle">Detects root split, likely canonical root, and root outlier nodes from live RPC state roots.</p>
        </div>
        <StatusPill label={auditStatus} state={auditState} />
      </div>
      <div className="state-audit-grid">
        {cards.map((card) => (
          <div className={`state-audit-card state-audit-card--${card.tone}`} key={card.label}>
            <p className="tiny-label">{card.label}</p>
            <strong className="mono">{card.value}</strong>
            <span>{card.detail}</span>
          </div>
        ))}
      </div>
      <div className="state-root-table">
        <div className="state-root-table__head">
          <div>
            <p className="tiny-label">Observed Root</p>
            <span className="copy-row copy-row--inline">
              <strong className="mono break-anywhere">{shortRoot(rootSummary)}</strong>
              {rootSummary ? <CopyButton value={rootSummary} label="Copy observed root" /> : null}
            </span>
          </div>
          <div>
            <p className="tiny-label">Comparison Height</p>
            <strong className="mono">{formatNumber(comparisonHeight)}</strong>
          </div>
          <div>
            <p className="tiny-label">Quorum</p>
            <strong className="mono">{quorum ? `${leaderCount}/${quorum}` : "-"}</strong>
          </div>
        </div>
        <div className="state-root-list">
          {stateRootChecks.length === 0 ? (
            <div className="state-root-row">
              <span className="muted-text">Waiting for state root reads...</span>
            </div>
          ) : (
            stateRootChecks.map((check) => {
              const isLagging = isLaggingAt(check, comparisonHeight);
              const isOutlier = Boolean(!isLagging && leadingRoot && check.root && check.root !== leadingRoot);
              const signalLabel = !check.online ? "offline" : isLagging ? "lagging" : isOutlier ? "outlier" : check.root ? "aligned" : "unreadable";
              const signalState = !check.online || isLagging || isOutlier ? "warn" : check.root ? "ok" : "warn";
              return (
                <div className={`state-root-row ${isOutlier ? "state-root-row--outlier" : ""}`} key={check.endpoint}>
                  <div>
                    <p className="tiny-label">Node</p>
                    <strong>{check.node}</strong>
                    <div className="muted-text mono">root height {formatNumber(check.height)}</div>
                    <div className="muted-text mono">latest {formatNumber(check.nodeHeight)}</div>
                  </div>
                  <div>
                    <p className="tiny-label">State Root</p>
                    <span className="copy-row copy-row--inline">
                      <span className="mono break-anywhere">{shortRoot(check.root)}</span>
                      {check.root ? <CopyButton value={check.root} label={`Copy ${check.node} root`} /> : null}
                    </span>
                  </div>
                  <div>
                    <p className="tiny-label">Signal</p>
                    <StatusPill label={signalLabel} state={signalState} />
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>
    </section>
  );
}

function MysticetiNodeGraph({
  blockHeight,
  configuredEndpoints,
  maxHeight,
  nodes,
  pendingTransactions,
  syncedNodes,
  totalTransactions,
}: {
  blockHeight: number | null;
  configuredEndpoints: RpcEndpoint[];
  maxHeight: number;
  nodes: NodeHealth[];
  pendingTransactions: number;
  syncedNodes: number;
  totalTransactions: number | null;
}) {
  const visualNodes = (
    nodes.length > 0
      ? nodes
      : configuredEndpoints.map(
        (endpoint): NodeHealth => ({
          endpoint,
          error: undefined,
          height: null,
          latencyMs: null,
          online: false,
          pendingTransactions: null,
          stateRoot: null,
          status: "loading",
          totalAccounts: null,
          totalTransactions: null,
        }),
      )
  ).slice(0, 8);
  const activeHeight = maxHeight || blockHeight || 0;
  const nodeCount = Math.max(visualNodes.length, 1);
  const positionedNodes = visualNodes.map((node, index) => {
    const angle = -90 + (360 / nodeCount) * index;
    const radians = (angle * Math.PI) / 180;
    const radius = index % 2 === 0 ? 40 : 32;
    const x = 50 + Math.cos(radians) * radius;
    const y = 50 + Math.sin(radians) * radius;
    const lag = maxHeight - (node.height ?? 0);
    const state = !node.online ? "down" : lag > 0 ? "warn" : "ok";
    const label = String(index + 1).padStart(2, "0");
    const status = !node.online ? node.status : lag > 0 ? `lag ${lag}` : "synced";
    return { label, node, state, status, x, y };
  });

  return (
    <section className="panel centauri-graph-panel">
      <div className="panel-head">
        <div>
          <h2 className="panel-title">Mysticeti Node Work</h2>
          <p className="panel-subtitle">Checkpoint work map across live RPC nodes</p>
        </div>
        <StatusPill label={`${syncedNodes}/${nodes.length || configuredEndpoints.length} synced`} />
      </div>

      <div className="centauri-graph">
        <div className="centauri-map" aria-label="Mysticeti node work graph">
          <span className="centauri-orbit centauri-orbit--outer" />
          <span className="centauri-orbit centauri-orbit--middle" />
          <span className="centauri-orbit centauri-orbit--inner" />
          <span className="centauri-packet centauri-packet--one" />
          <span className="centauri-packet centauri-packet--two" />
          <span className="centauri-label centauri-label--proposal">Propose</span>
          <span className="centauri-label centauri-label--vote">Vote</span>
          <span className="centauri-label centauri-label--commit">Commit</span>

          <svg className="centauri-edges" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
            {positionedNodes.map(({ node, state, x, y }) => (
              <line className={`centauri-edge centauri-edge--${state}`} key={node.endpoint.url} x1="50" x2={x} y1="50" y2={y} />
            ))}
          </svg>

          <div className="centauri-core">
            <Image src="/kariicon1.png" alt="" width={64} height={64} />
            <strong>Mysticeti</strong>
            <span className="mono">H {formatNumber(activeHeight)}</span>
          </div>

          {positionedNodes.map(({ label, node, state, status, x, y }) => (
            <div
              className={`centauri-node centauri-node--${state}`}
              key={node.endpoint.url}
              style={{ "--node-x": `${x}%`, "--node-y": `${y}%` } as CSSProperties}
              title={`${node.endpoint.name}: ${status}`}
            >
              <span>{label}</span>
              <small>{status}</small>
            </div>
          ))}
        </div>

        <div className="centauri-work">
          <div className="centauri-work-card">
            <p className="tiny-label">Committee</p>
            <strong className="mono">{syncedNodes}/{nodes.length || configuredEndpoints.length}</strong>
            <span>synced authorities</span>
          </div>
          <div className="centauri-work-card">
            <p className="tiny-label">Round Height</p>
            <strong className="mono">{formatNumber(activeHeight)}</strong>
            <span>latest observed checkpoint</span>
          </div>
          <div className="centauri-work-lanes">
            {[
              ["Propose", activeHeight ? 100 : 28],
              ["Vote", syncedNodes && nodeCount ? Math.round((syncedNodes / nodeCount) * 100) : 12],
              ["Commit", pendingTransactions === 0 ? 100 : 64],
            ].map(([label, value]) => (
              <div className="centauri-lane" key={label}>
                <div>
                  <p className="tiny-label">{label}</p>
                  <span className="mono">{value}%</span>
                </div>
                <i style={{ width: `${value}%` }} />
              </div>
            ))}
          </div>
          <div className="centauri-work-card centauri-work-card--accent">
            <p className="tiny-label centauri-work-label">Work Queue</p>
            <strong className="mono">{formatNumber(pendingTransactions)}</strong>
            <span>{formatNumber(totalTransactions)} committed txs</span>
          </div>
        </div>
      </div>
    </section>
  );
}

export default function Home() {
  const [search, setSearch] = useState("");
  const [tokenCount, setTokenCount] = useState<number | null>(null);
  const [blockHeight, setBlockHeight] = useState<number | null>(null);
  const [configuredEndpoints, setConfiguredEndpoints] = useState<RpcEndpoint[]>(RPC_ENDPOINTS);
  const [latestBlock, setLatestBlock] = useState<unknown>(null);
  const [nodes, setNodes] = useState<NodeHealth[]>([]);
  const [lastRecoveryMs, setLastRecoveryMs] = useState<number | null>(null);
  const [lastUpdated, setLastUpdated] = useState("");
  const [divergenceWindow, setDivergenceWindow] = useState<DivergenceScan[]>([]);
  const [stabilitySamples, setStabilitySamples] = useState<StabilitySample[]>([]);
  const [stateRootChecks, setStateRootChecks] = useState<StateRootCheck[]>([]);
  const offlineSinceRef = useRef<Map<string, number>>(new Map());
  const lastRootScanAtRef = useRef(0);
  const router = useRouter();

  const onlineNodes = nodes.filter((node) => node.online);
  const maxHeight = Math.max(0, ...nodes.map((node) => node.height ?? 0));
  const syncedAuthorityNodes = onlineNodes.filter((node) => (node.height ?? 0) >= maxHeight);
  const syncedNodes = syncedAuthorityNodes.length;
  const laggingNodes = onlineNodes.length - syncedNodes;
  const offlineNodes = nodes.length - onlineNodes.length;
  const totalTransactions = maxNullable(nodes.map((node) => node.totalTransactions));
  const totalAccounts = maxNullable(nodes.map((node) => node.totalAccounts));
  const pendingTransactions =
    maxNullable((syncedAuthorityNodes.length > 0 ? syncedAuthorityNodes : onlineNodes).map((node) => node.pendingTransactions)) ?? 0;

  const networkStatusLabel = useMemo(() => {
    if (nodes.length === 0) return "Loading network";
    if (onlineNodes.length === nodes.length && laggingNodes === 0) return "All nodes synced";
    if (onlineNodes.length === nodes.length) return "Nodes syncing";
    if (onlineNodes.length > 0) return "Partial outage";
    return "Network offline";
  }, [laggingNodes, nodes.length, onlineNodes.length]);

  useEffect(() => {
    async function fetchNetworkData() {
      const seedEndpoint = RPC_ENDPOINTS[0];
      const [tokensRes, heightRes, networkStatus] = await Promise.all([
        getTokens().catch(() => null),
        getBlockHeight().catch(() => null),
        getNetworkStatus(seedEndpoint?.url).catch(() => null),
      ]);

      const nextEndpoints =
        RPC_ENDPOINTS.length === 1 && seedEndpoint && networkStatus?.authorities?.length
          ? deriveAuthorityRpcEndpoints(seedEndpoint.url, networkStatus)
          : RPC_ENDPOINTS;
      const nodeResults = await Promise.all(nextEndpoints.map((endpoint) => getNodeHealth(endpoint)));
      const parsedHeight = typeof heightRes === "number" ? heightRes : Number(heightRes);
      const latestHeight = Number.isFinite(parsedHeight) ? parsedHeight : null;
      const onlineHeights = nodeResults
        .filter((node) => node.online && node.height !== null)
        .map((node) => node.height as number);
      const liveHeight = Math.max(0, latestHeight ?? 0, ...onlineHeights);
      const commonHeight = onlineHeights.length > 0 ? Math.min(...onlineHeights) : null;
      const blockRes =
        latestHeight === null
          ? null
          : await getFullBlock(latestHeight).catch(() => getBlock(latestHeight).catch(() => null));
      const now = Date.now();
      let nextStateRootChecks: StateRootCheck[] | null = null;
      let nextDivergenceWindow: DivergenceScan[] | null = null;
      if (commonHeight === null) {
        nextStateRootChecks = [];
        nextDivergenceWindow = [];
        lastRootScanAtRef.current = now;
      } else if (now - lastRootScanAtRef.current >= ROOT_SCAN_INTERVAL_MS) {
        const nodeByUrl = new Map(nodeResults.map((node) => [node.endpoint.url, node]));
        nextStateRootChecks = await readStateRootChecksAtHeight(commonHeight, nextEndpoints, nodeByUrl);
        const scanEndpoints = nextEndpoints.slice(0, ROOT_SCAN_ENDPOINT_LIMIT);
        const scanStart = Math.max(0, commonHeight - ROOT_SCAN_DEPTH + 1);
        nextDivergenceWindow = await Promise.all(
          Array.from({ length: commonHeight - scanStart + 1 }, (_, index) => scanStart + index).map(async (height) => ({
            checks: await readStateRootChecksAtHeight(height, scanEndpoints, nodeByUrl),
            height,
          })),
        );
        lastRootScanAtRef.current = now;
      }

      nodeResults.forEach((node) => {
        const key = node.endpoint.url;
        if (!node.online) {
          if (!offlineSinceRef.current.has(key)) offlineSinceRef.current.set(key, now);
          return;
        }

        const offlineSince = offlineSinceRef.current.get(key);
        if (offlineSince !== undefined) {
          setLastRecoveryMs(now - offlineSince);
          offlineSinceRef.current.delete(key);
        }
      });

      setTokenCount(asArray(tokensRes).length || null);
      setBlockHeight(latestHeight);
      setLatestBlock(blockRes);
      setConfiguredEndpoints(nextEndpoints);
      setNodes(nodeResults);
      if (nextDivergenceWindow !== null) setDivergenceWindow(nextDivergenceWindow);
      setStabilitySamples((current) =>
        [
          ...current,
          {
            height: liveHeight,
            online: nodeResults.filter((node) => node.online).length,
            pending: nodeResults.reduce((sum, node) => sum + (node.pendingTransactions ?? 0), 0),
            timestamp: now,
          },
        ].slice(-24),
      );
      if (nextStateRootChecks !== null) setStateRootChecks(nextStateRootChecks);
      setLastUpdated(new Date().toLocaleTimeString());
    }

    fetchNetworkData();
    const interval = window.setInterval(fetchNetworkData, NETWORK_POLL_INTERVAL_MS);
    return () => window.clearInterval(interval);
  }, []);

  function handleSearch() {
    const query = search.trim();
    if (!query) return;
    router.push(query.length > 44 ? `/tx?hash=${encodeURIComponent(query)}` : `/account?address=${encodeURIComponent(query)}`);
  }

  return (
    <div className="explorer-wrap">
      <section className="hero-section">
        <div className="hero-copy">
          <p className="eyebrow"><span /> {networkStatusLabel}</p>
          <h1>
            Explore<br />
            <span>Kanari.</span>
          </h1>
          <p className="hero-description">
            Track node health, transactions, token registry, accounts, and NFT collections across the Kanari event-driven ledger.
          </p>
          <div className="hero-actions">
            <Link className="button button--dark" href="/tx">
              Transactions <ArrowIcon />
            </Link>
            <Link className="button button--ghost" href="/account">
              Accounts <ArrowIcon />
            </Link>
          </div>
        </div>

        <div className="hero-visual">
          <div className="hero-sticker hero-sticker--top">LIVE<br />RPC</div>
          <NetworkGraphic />
          <div className="hero-sticker hero-sticker--bottom">OPEN<br />DATA</div>
        </div>
      </section>

      <section className="explorer-search-panel">
        <div>
          <p className="section-kicker">{networkStatusLabel}</p>
          <h2>Explorer search.</h2>
        </div>
        <SearchForm value={search} onChange={setSearch} onSubmit={handleSearch} placeholder="Search address or transaction hash" />
      </section>

      <section className="stat-grid explorer-stat-grid">
        <StatCard label="Nodes" value={`${onlineNodes.length}/${nodes.length || configuredEndpoints.length}`} detail={offlineNodes ? `${offlineNodes} offline` : `${laggingNodes} lagging`} />
        <StatCard label="Height" value={formatNumber(maxHeight || blockHeight)} detail="Highest reported checkpoint height" />
        <StatCard label="Transactions" value={formatNumber(totalTransactions)} detail={`${formatNumber(pendingTransactions)} pending on synced authority`} />
        <StatCard label="Tokens" value={formatNumber(tokenCount)} detail={`${formatNumber(totalAccounts)} accounts indexed`} />
      </section>

      <MysticetiNodeGraph
        blockHeight={blockHeight}
        configuredEndpoints={configuredEndpoints}
        maxHeight={maxHeight}
        nodes={nodes}
        pendingTransactions={pendingTransactions}
        syncedNodes={syncedNodes}
        totalTransactions={totalTransactions}
      />

      <ConsensusStabilityPanel
        blockHeight={blockHeight}
        lastRecoveryMs={lastRecoveryMs}
        nodes={nodes}
        pendingTransactions={pendingTransactions}
        samples={stabilitySamples}
        stateRootChecks={stateRootChecks}
      />

      <StateDivergenceAudit divergenceWindow={divergenceWindow} stateRootChecks={stateRootChecks} />

      <section className="content-grid">
        <Panel
          title="Node Status"
          subtitle={`Refreshes every ${NETWORK_POLL_LABEL}${lastUpdated ? `, last ${lastUpdated}` : ""}`}
          action={<StatusPill label={`${syncedNodes} synced`} state={offlineNodes ? "warn" : "ok"} />}
        >
          <div className="data-list">
            {(nodes.length > 0
              ? nodes
              : configuredEndpoints.map(
                (endpoint): NodeHealth => ({
                  endpoint,
                  error: undefined,
                  height: null,
                  latencyMs: null,
                  online: false,
                  pendingTransactions: null,
                  stateRoot: null,
                  status: "loading",
                  totalAccounts: null,
                  totalTransactions: null,
                }),
              )
            ).map((node) => {
              const lag = maxHeight - (node.height ?? 0);
              return (
                <div className="data-row" key={node.endpoint.url}>
                  <div className="primary-text">
                    <div>
                      <span className={`dot ${node.online ? "" : "dot--down"}`} /> {node.endpoint.name}
                    </div>
                    <div className="muted-text mono">{shortUrl(node.endpoint.url)}</div>
                  </div>
                  <div>
                    <p className="tiny-label">Status</p>
                    <StatusPill label={!node.online ? node.status : lag > 0 ? `Lag ${lag}` : "Synced"} state={!node.online ? "down" : lag > 0 ? "warn" : "ok"} />
                  </div>
                  <div>
                    <p className="tiny-label">Height</p>
                    <span className="mono">{formatNumber(node.height)}</span>
                  </div>
                  <div>
                    <p className="tiny-label">Latency</p>
                    <span className="mono">{node.latencyMs === null ? "-" : `${node.latencyMs} ms`}</span>
                  </div>
                </div>
              );
            })}
          </div>
        </Panel>

        <Panel
          title="Latest Checkpoint"
          subtitle={blockHeight === null ? "Waiting for checkpoint height" : `Height ${formatNumber(blockHeight)}`}
          action={<StatusPill label={latestBlock ? "Loaded" : "Pending"} state={latestBlock ? "ok" : "warn"} />}
        >
          <div className="block-summary">
            <div className="block-summary__top">
              <div>
                <p className="tiny-label">Height</p>
                <strong className="mono block-summary__height">{readString(latestBlock, "height", formatNumber(blockHeight))}</strong>
              </div>
              <div>
                <p className="tiny-label">Transactions</p>
                <strong className="mono">
                  {readString(latestBlock, "tx_count", readString(latestBlock, "transaction_count", readString(latestBlock, "transactions_len", "-")))}
                </strong>
              </div>
            </div>
            <div>
              <p className="tiny-label">Hash</p>
              <p className="mono muted-text block-summary__hash">{readString(latestBlock, "hash")}</p>
            </div>
            <div>
              <p className="tiny-label">State Root</p>
              <p className="mono muted-text block-summary__hash">{readString(latestBlock, "state_root")}</p>
            </div>
          </div>
        </Panel>
      </section>
    </div>
  );
}

