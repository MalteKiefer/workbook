/**
 * Parses a dotted-quad IPv4 address into its 32-bit unsigned integer form,
 * or `null` if `ip` isn't a valid IPv4 address (four octets, each 0-255).
 */
export function ipToInt(ip: string): number | null {
  const parts = ip.trim().split(".");
  if (parts.length !== 4) return null;
  const octets = parts.map((p) => Number(p));
  if (octets.some((n) => !Number.isInteger(n) || n < 0 || n > 255)) return null;
  return ((octets[0] << 24) | (octets[1] << 16) | (octets[2] << 8) | octets[3]) >>> 0;
}

/**
 * Whether `ip` falls inside `cidr` (e.g. "192.168.1.0/24"). Returns `false`
 * (never throws) for any malformed input on either side -- this is a
 * display-grouping helper, not a validator; an unparseable network or IP
 * simply doesn't match anything rather than crashing the view.
 */
export function cidrContains(cidr: string, ip: string): boolean {
  const [base, prefixStr] = cidr.split("/");
  const prefix = Number(prefixStr);
  if (!Number.isInteger(prefix) || prefix < 0 || prefix > 32) return false;
  const baseInt = ipToInt(base);
  const ipInt = ipToInt(ip);
  if (baseInt === null || ipInt === null) return false;
  if (prefix === 0) return true;
  const mask = prefix === 32 ? 0xffffffff : (~0 << (32 - prefix)) >>> 0;
  return (baseInt & mask) === (ipInt & mask);
}
