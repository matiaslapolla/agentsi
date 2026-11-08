const LOCAL_HOST = /^(?:127\.0\.0\.1|localhost|\[::1\])(?::\d{1,5})?$/i;

/** True when the `Host` header names this machine's loopback, on any port. A DNS-rebinding page sends its own hostname. */
export const isLocalHost = (host: string | null): boolean => host !== null && LOCAL_HOST.test(host);
