import { browser } from '$app/environment';
import { redirect } from '@sveltejs/kit';
import { fetchAuthCallback } from '$lib/api';
import { setToken } from '$lib/auth';
import { base } from '$app/paths';

export const ssr = false;

export async function load({ url }: { url: URL }) {
	if (!browser) return {};

	const code = url.searchParams.get('code');
	const state = url.searchParams.get('state');

	if (!code || !state) {
		redirect(302, base || '/');
	}

	try {
		const { token } = await fetchAuthCallback(code, state);
		setToken(token);
		redirect(302, `${base}/dashboard`);
	} catch (e) {
		if ((e as { status?: number }).status === 302) throw e;
		return { error: String(e) };
	}

	return {};
}
