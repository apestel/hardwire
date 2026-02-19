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
