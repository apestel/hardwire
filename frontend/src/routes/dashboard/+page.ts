import {
	fetchDownloadStats,
	fetchDownloadsByPeriod,
	fetchRecentDownloads,
	fetchStatusDistribution,
} from '$lib/api';

export const ssr = false;

export async function load() {
	const [stats, byPeriod, recent, statusDist] = await Promise.all([
		fetchDownloadStats(),
		fetchDownloadsByPeriod('month', 30),
		fetchRecentDownloads(50),
		fetchStatusDistribution(),
	]);
	return { stats, byPeriod, recent, statusDist };
}
