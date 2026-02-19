<script lang="ts">
	let {
		period = $bindable<'day' | 'week' | 'month'>('month'),
		onchange,
	}: {
		period: 'day' | 'week' | 'month';
		onchange?: () => void;
	} = $props();
</script>

<select
	bind:value={period}
	{onchange}
	class="bg-gray-800 border border-gray-700 rounded px-2 py-1 text-sm text-white focus:outline-none focus:ring-1 focus:ring-blue-500"
>
	<option value="day">Day</option>
	<option value="week">Week</option>
	<option value="month">Month</option>
</select>
