export interface ProxyEnvironment {
  readonly HTTP_PROXY: string;
  readonly HTTPS_PROXY: string;
  readonly ALL_PROXY: string;
  readonly NO_PROXY: string;
  readonly http_proxy: string;
  readonly https_proxy: string;
  readonly all_proxy: string;
  readonly no_proxy: string;
}

export function buildProxyEnv(proxyUrl: string, noProxy: string): ProxyEnvironment {
  return {
    HTTP_PROXY: proxyUrl,
    HTTPS_PROXY: proxyUrl,
    ALL_PROXY: proxyUrl,
    NO_PROXY: noProxy,
    http_proxy: proxyUrl,
    https_proxy: proxyUrl,
    all_proxy: proxyUrl,
    no_proxy: noProxy
  };
}
