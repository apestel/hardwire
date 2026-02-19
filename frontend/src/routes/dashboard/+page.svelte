<script lang="ts">
	import { fetchDownloadsByPeriod } from '$lib/api';
	import StatsCard from '$lib/components/StatsCard.svelte';
	import DownloadsChart from '$lib/components/DownloadsChart.svelte';
	import RecentDownloadsTable from '$lib/components/RecentDownloadsTable.svelte';
	import PeriodSelector from '$lib/components/PeriodSelector.svelte';
	import type { DownloadsByPeriod } from '$lib/types';

	let { data } = $props();

	let period = $state<'day' | 'week' | 'month'>('month');
	// eslint-disable-next-line svelte/state_referenced_locally
	let chartData = $state<DownloadsByPeriod>(data.byPeriod);
	let fromDate = $state('');
	let toDate = $state('');

	function formatSize(bytes: number): string {
		if (bytes < 1024) return `${bytes} B`;
		if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
		if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
		return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
	}

	function formatTime(seconds: number | null): string {
		if (seconds === null) return 'N/A';
		if (seconds < 60) return `${Math.round(seconds)}s`;
		return `${Math.round(seconds / 60)}m`;
	}

	async function refreshChart() {
		const raw = await fetchDownloadsByPeriod(period, 60);
		if (!fromDate && !toDate) {
			chartData = raw;
			return;
		}
		const from = fromDate ? new Date(fromDate).getTime() / 1000 : 0;
		const to = toDate ? new Date(toDate).getTime() / 1000 + 86400 : Infinity;
		chartData = {
			...raw,
			data: raw.data.filter(() => {
				// Client-side date filtering (approximate — server groups by period label)
				return true;
			}),
		};
		// Simple label-based filtering
		chartData = {
			...raw,
			data: raw.data.filter((d) => {
				const ts = new Date(d.date).getTime() / 1000;
				return ts >= from && ts <= to;
			}),
		};
	}

	$effect(() => {
		// Re-fetch when period changes
		void period;
		refreshChart();
	});
</script>

<div class="space-y-8">
	<h1 class="text-2xl font-bold text-white">Dashboard</h1>

	<div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
		<StatsCard label="Total Downloads" value={String(data.stats.total_downloads)} />
		<StatsCard label="Total Size" value={formatSize(data.stats.total_size)} />
		<StatsCard label="Success Rate" value={`${data.stats.success_rate.toFixed(1)}%`} />
		<StatsCard
			label="Avg Download Time"
			value={formatTime(data.stats.average_download_time)}
		/>
	</div>

	<div class="bg-gray-900 border border-gray-800 rounded-xl p-6">
		<div class="flex items-center justify-between mb-4 flex-wrap gap-3">
			<h2 class="text-lg font-semibold text-white">Downloads Over Time</h2>
			<div class="flex items-center gap-3 flex-wrap">
				<input
					type="date"
					bind:value={fromDate}
					onchange={refreshChart}
					class="bg-gray-800 border border-gray-700 rounded px-2 py-1 text-sm text-white"
				/>
				<span class="text-gray-500 text-sm">to</span>
				<input
					type="date"
					bind:value={toDate}
					onchange={refreshChart}
					class="bg-gray-800 border border-gray-700 rounded px-2 py-1 text-sm text-white"
				/>
				<PeriodSelector bind:period onchange={refreshChart} />
			</div>
		</div>
		<DownloadsChart data={chartData} />
	</div>

	<div class="bg-gray-900 border border-gray-800 rounded-xl p-6">
		<h2 class="text-lg font-semibold text-white mb-4">Recent Downloads</h2>
		<RecentDownloadsTable rows={data.recent} />
	</div>
</div>
