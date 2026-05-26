import * as net from 'net';
import { LocalProxy, ProxyScheme, RemoteProxyConfig } from './types';

const ENV_PROXY_KEYS = ['HTTPS_PROXY', 'HTTP_PROXY', 'ALL_PROXY', 'https_proxy', 'http_proxy', 'all_proxy'];

export async function detectLocalProxy(config: RemoteProxyConfig): Promise<LocalProxy | undefined> {
  if (config.localProxyMode === 'manual') {
    return parseProxyUrl(config.localProxyUrl, 'manual setting');
  }

  if (config.localProxyMode === 'auto') {
    const manual = parseProxyUrl(config.localProxyUrl, 'manual setting');
    if (manual) {
      return manual;
    }
  }

  const fromEnv = detectProxyFromEnv();
  if (fromEnv) {
    return fromEnv;
  }

  if (config.localProxyMode === 'env') {
    return undefined;
  }

  for (const port of config.localProxyAutoPorts) {
    for (const host of config.localProxyHosts) {
      if (await canConnect(host, port, 350)) {
        const scheme = config.localProxyDefaultScheme;
        return parseProxyUrl(`${scheme}://${host}:${port}`, `port probe ${host}:${port}`);
      }
    }
  }

  return undefined;
}

export function makeRemoteProxyUrl(local: LocalProxy, remoteBindHost: string, remotePort: number): string {
  const url = new URL(local.url);
  url.hostname = remoteBindHost;
  url.port = String(remotePort);
  return url.toString();
}

export function parseProxyUrl(rawValue: string, source: string): LocalProxy | undefined {
  const normalized = normalizeProxyUrl(rawValue);
  if (!normalized) {
    return undefined;
  }

  let parsed: URL;
  try {
    parsed = new URL(normalized);
  } catch {
    return undefined;
  }

  const scheme = parsed.protocol.replace(':', '') as ProxyScheme;
  if (!['http', 'https', 'socks4', 'socks5'].includes(scheme)) {
    return undefined;
  }

  const port = parsed.port ? Number(parsed.port) : defaultPortForScheme(scheme);
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    return undefined;
  }

  return {
    url: parsed.toString(),
    scheme,
    host: parsed.hostname || '127.0.0.1',
    port,
    source
  };
}

export async function findProbeCandidates(config: RemoteProxyConfig): Promise<LocalProxy[]> {
  const candidates: LocalProxy[] = [];

  const manual = parseProxyUrl(config.localProxyUrl, 'manual setting');
  if (manual) {
    candidates.push(manual);
  }

  const env = detectProxyFromEnv();
  if (env && !candidates.some((candidate) => candidate.url === env.url)) {
    candidates.push(env);
  }

  for (const port of config.localProxyAutoPorts) {
    for (const host of config.localProxyHosts) {
      if (await canConnect(host, port, 350)) {
        const candidate = parseProxyUrl(`${config.localProxyDefaultScheme}://${host}:${port}`, `port probe ${host}:${port}`);
        if (candidate && !candidates.some((existing) => existing.url === candidate.url)) {
          candidates.push(candidate);
        }
      }
    }
  }

  return candidates;
}

function detectProxyFromEnv(): LocalProxy | undefined {
  for (const key of ENV_PROXY_KEYS) {
    const value = process.env[key];
    const proxy = parseProxyUrl(value ?? '', `environment ${key}`);
    if (proxy) {
      return proxy;
    }
  }

  return undefined;
}

function normalizeProxyUrl(rawValue: string): string | undefined {
  const trimmed = rawValue.trim();
  if (!trimmed) {
    return undefined;
  }

  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed)) {
    return trimmed;
  }

  if (/^[\w.-]+:\d+$/.test(trimmed)) {
    return `http://${trimmed}`;
  }

  return undefined;
}

function defaultPortForScheme(scheme: ProxyScheme): number {
  if (scheme === 'https') {
    return 443;
  }

  if (scheme === 'socks4' || scheme === 'socks5') {
    return 1080;
  }

  return 80;
}

function canConnect(host: string, port: number, timeoutMs: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = net.connect({ host, port });
    let settled = false;

    const settle = (result: boolean) => {
      if (settled) {
        return;
      }
      settled = true;
      socket.destroy();
      resolve(result);
    };

    socket.setTimeout(timeoutMs);
    socket.once('connect', () => settle(true));
    socket.once('timeout', () => settle(false));
    socket.once('error', () => settle(false));
  });
}
