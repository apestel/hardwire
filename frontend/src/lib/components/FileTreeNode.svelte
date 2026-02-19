<script lang="ts">
	import type { FileInfo } from '$lib/types';
	import FileTreeNode from './FileTreeNode.svelte';

	let {
		node,
		selected,
		toggleSelect,
		depth,
	}: {
		node: FileInfo;
		selected: Set<string>;
		toggleSelect: (path: string) => void;
		depth: number;
	} = $props();

	let open = $state(false);

	function formatSize(bytes?: number) {
		if (!bytes) return '';
		if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
		return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
	}

	function formatDate(ts?: number) {
		if (!ts) return '';
		return new Date(ts * 1000).toLocaleDateString(undefined, {
			year: 'numeric',
			month: 'short',
			day: 'numeric',
		});
	}
</script>

<li>
	<div
		class="flex items-center gap-2 py-1 px-2 rounded hover:bg-gray-800 cursor-pointer select-none"
		style="padding-left: {8 + depth * 16}px"
		onclick={() => (node.is_dir ? (open = !open) : toggleSelect(node.full_path))}
		role="button"
		tabindex="0"
		onkeydown={(e) => e.key === 'Enter' && (node.is_dir ? (open = !open) : toggleSelect(node.full_path))}
	>
		{#if node.is_dir}
			<span class="text-yellow-400 text-xs w-3 text-center shrink-0">{open ? '▼' : '▶'}</span>
			<span class="text-yellow-300">{node.name}/</span>
		{:else}
			<input
				type="checkbox"
				checked={selected.has(node.full_path)}
				onclick={(e) => { e.stopPropagation(); toggleSelect(node.full_path); }}
				class="rounded border-gray-600 bg-gray-800 text-blue-500 shrink-0"
			/>
			<span class="text-gray-200 flex-1 truncate">{node.name}</span>
			<span class="text-gray-600 text-xs shrink-0 pl-4 w-24 text-right hidden sm:block">{formatDate(node.created_at)}</span>
			<span class="text-gray-600 text-xs shrink-0 pl-4 w-24 text-right hidden sm:block">{formatDate(node.modified_at)}</span>
			<span class="text-gray-500 text-xs shrink-0 pl-4 w-16 text-right">{formatSize(node.size)}</span>
		{/if}
	</div>

	{#if node.is_dir && open && node.children}
		<ul>
			{#each node.children as child (child.full_path)}
				<FileTreeNode node={child} {selected} {toggleSelect} depth={depth + 1} />
			{/each}
		</ul>
	{/if}
</li>
