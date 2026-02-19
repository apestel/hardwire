import { browser } from '$app/environment';
import { redirect } from '@sveltejs/kit';
import { setToken } from '$lib/auth';
import { base } from '$app/paths';

export const ssr = false;

export async function load({ url }: { url: URL }) {
	if (!browser) return {};

	const token = url.searchParams.get('token');

	if (!token) {
		return { error: 'No token received from authentication server.' };
	}

	setToken(token);
	redirect(302, `${base}/dashboard`);
}
