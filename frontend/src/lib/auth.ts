const TOKEN_KEY = 'hardwire_admin_token';

export function getToken(): string | null {
	return localStorage.getItem(TOKEN_KEY);
}

export function setToken(token: string): void {
	localStorage.setItem(TOKEN_KEY, token);
}

export function clearToken(): void {
	localStorage.removeItem(TOKEN_KEY);
}

export function isAuthenticated(): boolean {
	const token = getToken();
	if (!token) return false;
	try {
		const payload = JSON.parse(atob(token.split('.')[1]));
		return payload.exp > Date.now() / 1000;
	} catch {
		return false;
	}
}

/// Asks the backend whether the stored token is still valid. The client-side
/// `isAuthenticated` check only looks at the (unverifiable) `exp` claim, so a
/// forged or revoked token must be caught server-side — this call does that.
export async function verifyToken(): Promise<boolean> {
	const token = getToken();
	if (!token) return false;
	try {
		const res = await fetch('/admin/api/users', {
			headers: { Authorization: `Bearer ${token}` }
		});
		return res.ok;
	} catch {
		// Network error: don't log the user out on a transient outage.
		return true;
	}
}
