<script>
    import { createEventDispatcher } from "svelte";
    export let currentPath = [];
    const dispatch = createEventDispatcher();

    function navigateTo(index) {
        const newPath = $currentPath.slice(0, index + 1);
        console.log("Breadcrumb newPath: " + newPath);
        dispatch("change", newPath);
    }
    console.log($currentPath);
</script>

<nav>
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <span on:click={() => navigateTo(-1)}> / </span>
    {#each $currentPath as part, index}
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <span on:click={() => navigateTo(index)}>{part}</span>
        {#if index < $currentPath.length - 1}
            <span> / </span>
        {/if}
    {/each}
</nav>

<style>
    nav {
        margin-bottom: 1rem;
    }

    span {
        cursor: pointer;
        color: blue;
    }

    span:hover {
        text-decoration: underline;
    }
</style>
