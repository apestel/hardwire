import { fetchFiles } from '$lib/api';

export const ssr = false;

export async function load() {
	const files = await fetchFiles();
	return { files };
}
