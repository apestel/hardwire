import { fetchFiles } from '$lib/api';

export const ssr = false;

export async function load({ depends }: { depends: (dep: string) => void }) {
	depends('app:files');
	const files = await fetchFiles();
	return { files };
}
