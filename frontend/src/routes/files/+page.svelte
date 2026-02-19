<script lang="ts">
	import FileTree from '$lib/components/FileTree.svelte';
	import { notifications } from '$lib/stores/notifications';
	import { createSharedLink, createTask, getTaskStatus, rescanFiles, fetchFiles } from '$lib/api';
	import type { FileInfo } from '$lib/types';

	let { data } = $props();

	let selected = $state<Set<string>>(new Set());
	let files = $state(data.files);
	let scanning = $state(false);

	type SortField = 'name' | 'modified_at' | 'created_at' | 'size';
	type SortDir = 'asc' | 'desc';

	let sortField = $state<SortField>('name');
	let sortDir = $state<SortDir>('asc');

	function sortNodes(nodes: FileInfo[]): FileInfo[] {
		return [...nodes]
			.sort((a, b) => {
				// Directories always first
				if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;

				let cmp = 0;
				if (sortField === 'name') {
					cmp = a.name.localeCompare(b.name);
				} else if (sortField === 'size') {
					cmp = (a.size ?? 0) - (b.size ?? 0);
				} else if (sortField === 'modified_at') {
					cmp = (a.modified_at ?? 0) - (b.modified_at ?? 0);
				} else if (sortField === 'created_at') {
					cmp = (a.created_at ?? 0) - (b.created_at ?? 0);
				}
				return sortDir === 'asc' ? cmp : -cmp;
			})
			.map((n) =>
				n.children ? { ...n, children: sortNodes(n.children) } : n,
			);
	}

	let sortedFiles = $derived(sortNodes(files));

	function toggleSort(field: SortField) {
		if (sortField === field) {
			sortDir = sortDir === 'asc' ? 'desc' : 'asc';
		} else {
			sortField = field;
			sortDir = 'asc';
		}
	}

	function toggleSelect(path: string) {
		const next = new Set(selected);
		if (next.has(path)) next.delete(path);
		else next.add(path);
		selected = next;
	}

	async function handleCreateShareLinks() {
		const paths = [...selected];
		const urls: string[] = [];
		for (const path of paths) {
			const link = await createSharedLink(path);
			urls.push(link.url);
		}
		await navigator.clipboard.writeText(urls.join('\n'));
		notifications.add({
			kind: 'success',
			message: `${urls.length} share link(s) copied to clipboard`,
			dismissible: true,
			autoDismissMs: 5000,
		});
	}

	async function handleRescan() {
		scanning = true;
		try {
			await rescanFiles();
			await new Promise((r) => setTimeout(r, 1500));
			files = await fetchFiles();
			selected = new Set();
		} finally {
			scanning = false;
		}
	}

	async function handleCreateArchive() {
		const filePaths = [...selected];
		const outputName = `archive-${Date.now()}`;

		const { task_id } = await createTask({
			type: 'CreateArchive',
			data: {
				files: filePaths,
				output_path: outputName,
			},
		});

		const notifId = notifications.add({
			kind: 'progress',
			message: 'Creating archive...',
			progress: 0,
			taskId: task_id,
			dismissible: false,
		});

		const interval = setInterval(async () => {
			const task = await getTaskStatus(task_id);
			notifications.updateProgress(notifId, task.progress, `Archiving... ${task.progress}%`);

			if (task.status === 'Completed') {
				clearInterval(interval);
				if (task.archive_path) {
					const link = await createSharedLink(task.archive_path);
					await navigator.clipboard.writeText(link.url);
				}
				notifications.complete(notifId, 'Archive ready — share link copied to clipboard');
			} else if (task.status === 'Failed') {
				clearInterval(interval);
				notifications.error(notifId, `Archive failed: ${task.error ?? 'unknown error'}`);
			}
		}, 2000);
	}

	const sortLabels: Record<SortField, string> = {
		name: 'Name',
		modified_at: 'Modified',
		created_at: 'Created',
		size: 'Size',
	};
</script>

<div class="space-y-4">
	<div class="flex items-center justify-between flex-wrap gap-4">
		<h1 class="text-2xl font-bold text-white">Files</h1>
		<div class="flex items-center gap-3 flex-wrap">
			<button
				onclick={handleRescan}
				disabled={scanning}
				class="flex items-center gap-1.5 bg-gray-700 hover:bg-gray-600 disabled:opacity-50 disabled:cursor-not-allowed text-white text-sm px-3 py-2 rounded-lg transition-colors"
			>
				<svg class="size-4 {scanning ? 'animate-spin' : ''}" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke="currentColor">
					<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
				</svg>
				{scanning ? 'Scanning…' : 'Refresh'}
			</button>
			{#if selected.size > 0}
				<span class="text-gray-400 text-sm">{selected.size} file(s) selected</span>
				<button
					onclick={handleCreateShareLinks}
					class="bg-blue-600 hover:bg-blue-700 text-white text-sm px-4 py-2 rounded-lg transition-colors"
				>
					Create Share Links
				</button>
				<button
					onclick={handleCreateArchive}
					class="bg-purple-600 hover:bg-purple-700 text-white text-sm px-4 py-2 rounded-lg transition-colors"
				>
					Create 7z Archive
				</button>
			{/if}
		</div>
	</div>

	<div class="bg-gray-900 border border-gray-800 rounded-xl p-4">
		<div class="flex items-center gap-1 mb-3 text-xs text-gray-500">
			<span>Sort:</span>
			{#each Object.entries(sortLabels) as [field, label]}
				<button
					onclick={() => toggleSort(field as SortField)}
					class="flex items-center gap-0.5 px-2 py-0.5 rounded transition-colors
						{sortField === field
							? 'bg-gray-700 text-white'
							: 'hover:bg-gray-800 text-gray-400 hover:text-white'}"
				>
					{label}
					{#if sortField === field}
						<span>{sortDir === 'asc' ? '↑' : '↓'}</span>
					{/if}
				</button>
			{/each}
		</div>

		{#if sortedFiles.length === 0}
			<p class="text-gray-500 text-sm text-center py-8">
				No files indexed yet. Files will appear once the indexer has scanned the data directory.
			</p>
		{:else}
			<FileTree nodes={sortedFiles} {selected} {toggleSelect} />
		{/if}
	</div>
</div>
