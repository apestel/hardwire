<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import {
		Chart,
		BarController,
		BarElement,
		CategoryScale,
		LinearScale,
		Tooltip,
		Legend,
	} from 'chart.js';
	import type { DownloadsByPeriod } from '$lib/types';

	Chart.register(BarController, BarElement, CategoryScale, LinearScale, Tooltip, Legend);

	let { data }: { data: DownloadsByPeriod } = $props();

	let canvas: HTMLCanvasElement;
	let chart: Chart | null = null;

	function buildChart() {
		if (chart) chart.destroy();
		chart = new Chart(canvas, {
			type: 'bar',
			data: {
				labels: data.data.map((d) => d.date),
				datasets: [
					{
						label: 'Downloads',
						data: data.data.map((d) => d.count),
						backgroundColor: '#3b82f6',
						borderRadius: 4,
					},
				],
			},
			options: {
				responsive: true,
				plugins: {
					legend: { display: false },
					tooltip: {
						callbacks: {
							afterLabel: (ctx) => {
								const entry = data.data[ctx.dataIndex];
								const mb = (entry.size / 1024 / 1024).toFixed(1);
								return `Size: ${mb} MB`;
							},
						},
					},
				},
				scales: {
					x: {
						ticks: { color: '#9ca3af' },
						grid: { color: '#1f2937' },
					},
					y: {
						ticks: { color: '#9ca3af', stepSize: 1 },
						grid: { color: '#1f2937' },
						beginAtZero: true,
					},
				},
			},
		});
	}

	onMount(() => buildChart());
	onDestroy(() => chart?.destroy());

	$effect(() => {
		// Re-build chart when data changes
		void data;
		if (canvas) buildChart();
	});
</script>

<canvas bind:this={canvas} class="w-full max-h-72"></canvas>
