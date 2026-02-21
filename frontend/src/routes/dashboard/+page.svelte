<script lang="ts">
	import { fetchDownloadsByPeriod, fetchRecentDownloads } from '$lib/api';
	import StatsCard from '$lib/components/StatsCard.svelte';
	import DownloadsChart from '$lib/components/DownloadsChart.svelte';
	import RecentDownloadsTable from '$lib/components/RecentDownloadsTable.svelte';
	import PeriodSelector from '$lib/components/PeriodSelector.svelte';
	import type { DownloadRecord, DownloadsByPeriod } from '$lib/types';

	let { data } = $props();

	let period = $state<'day' | 'week' | 'month'>('month');
	// eslint-disable-next-line svelte/state_referenced_locally
	let chartData = $state<DownloadsByPeriod>(data.byPeriod);
	let recentRows = $state<DownloadRecord[]>(data.recent);
	let fromDate = $state('');
	let toDate = $state('');
	let refreshingTable = $state(false);
	let refreshingChart = $state(false);

	let canFilter = $derived(fromDate !== '' && toDate !== '');

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
		refreshingChart = true;
		try {
			const raw = await fetchDownloadsByPeriod(period, 60);
			if (!fromDate || !toDate) {
				chartData = raw;
				return;
			}
			const from = new Date(fromDate).getTime() / 1000;
			const to = new Date(toDate).getTime() / 1000 + 86400;
			chartData = {
				...raw,
				data: raw.data.filter((d) => {
					const ts = new Date(d.date).getTime() / 1000;
					return ts >= from && ts <= to;
				}),
			};
		} finally {
			refreshingChart = false;
		}
	}

	async function refreshTable() {
		refreshingTable = true;
		try {
			recentRows = await fetchRecentDownloads(100);
		} finally {
			refreshingTable = false;
		}
	}

	$effect(() => {
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

	<div class="relative bg-gray-900 border border-gray-800 rounded-xl p-6">
		{#if refreshingChart}
			<div class="absolute inset-0 z-10 flex items-center justify-center rounded-xl bg-gray-900/60 backdrop-blur-sm">
				<svg class="size-6 animate-spin text-blue-500" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
					<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
					<path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"></path>
				</svg>
			</div>
		{/if}
		<div class="flex items-center justify-between mb-4 flex-wrap gap-3">
			<h2 class="text-lg font-semibold text-white">Downloads Over Time</h2>
			<div class="flex items-center gap-3 flex-wrap">
				<input
					type="date"
					bind:value={fromDate}
					class="bg-gray-800 border border-gray-700 rounded px-2 py-1 text-sm text-white"
				/>
				<span class="text-gray-500 text-sm">to</span>
				<input
					type="date"
					bind:value={toDate}
					class="bg-gray-800 border border-gray-700 rounded px-2 py-1 text-sm text-white"
				/>
				{#if canFilter}
					<button
						onclick={refreshChart}
						disabled={refreshingChart}
						class="flex items-center gap-1.5 bg-blue-600 hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed text-white text-sm px-3 py-1.5 rounded-lg transition-colors"
					>
						<svg class="size-3.5 {refreshingChart ? 'animate-spin' : ''}" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke="currentColor">
							<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
						</svg>
						Apply
					</button>
				{/if}
				<PeriodSelector bind:period onchange={refreshChart} />
			</div>
		</div>
		<DownloadsChart data={chartData} />
	</div>

	<div class="relative bg-gray-900 border border-gray-800 rounded-xl p-6">
		{#if refreshingTable}
			<div class="absolute inset-0 z-10 flex items-center justify-center rounded-xl bg-gray-900/60 backdrop-blur-sm">
				<svg class="size-6 animate-spin text-blue-500" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
					<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
					<path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"></path>
				</svg>
			</div>
		{/if}
		<div class="flex items-center justify-between mb-4">
			<h2 class="text-lg font-semibold text-white">Recent Downloads</h2>
			<button
				onclick={refreshTable}
				disabled={refreshingTable}
				class="flex items-center gap-1.5 bg-gray-700 hover:bg-gray-600 disabled:opacity-50 text-white text-sm px-3 py-1.5 rounded-lg transition-colors"
			>
				<svg class="size-3.5 {refreshingTable ? 'animate-spin' : ''}" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke="currentColor">
					<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
				</svg>
				Refresh
			</button>
		</div>
		<RecentDownloadsTable rows={recentRows} />
	</div>
</div>
