import cdns from "../../assets/static-cdns.json";

const hosts = new Set(cdns.hosts);
export function requestDelay(url: string, configured: number, asset: boolean) {
  return asset && hosts.has(new URL(url).hostname.replace(/\.+$/, ""))
    ? 0
    : configured;
}
