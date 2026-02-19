import { browser } from '$app/environment';
import { redirect } from '@sveltejs/kit';
import { isAuthenticated } from '$lib/auth';
import { base } from '$app/paths';

export const prerender = false;
export const ssr = false;

export function load({ url }: { url: URL }) {
	if (!browser) return {};

	const path = url.pathname;
	const isAuthRoute = path.startsWith(`${base}/auth/`);
	const isLoginPage = path === base || path === `${base}/`;

	if (!isAuthenticated() && !isAuthRoute && !isLoginPage) {
		redirect(302, base || '/');
	}
	if (isAuthenticated() && isLoginPage) {
		redirect(302, `${base}/dashboard`);
	}
	return {};
}
