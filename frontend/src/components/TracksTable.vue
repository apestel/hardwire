<script setup>
import { reactive, ref } from 'vue'

// give each todo a unique id
let id = 0

const downloading = reactive({ status: false })
const search = ref('')
const tracks = ref([
])

async function searchAlbum() {
    console.log("Search: " + `${search.value}`)
    tracks.value = null
    downloading.status = false
    const res = await fetch(
        `/api/search_album`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ album_name: `${search.value}` }) })
    tracks.value = await res.json()
    console.log(tracks)
}

async function downloadTracks() {
    downloading.status = true
    var tracks_to_download = []
    tracks.value.forEach(track => {
        tracks_to_download.push(track.id)
    });

    const res = await fetch('/api/request_tracks_download', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ tracks_id: tracks_to_download }) })
    console.log(tracks_to_download)
}

async function getTracksStatus() {
    var tracks_to_refresh = []
    var tracks_url
    tracks.value.forEach(track => {
        tracks_to_refresh.push(track.id)
    });

    if (tracks_to_refresh.length > 0) {
        const res = await fetch(
            `/api/get_tracks_status`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ tracks_id: tracks_to_refresh }) })
        tracks_url = await res.json()
        tracks_url.forEach(u => {
            tracks.value.forEach(track => {
                if (track.id == u.track_id) {
                    console.log(track.url)
                    track.url = u.url
                }
            })
        })
    }
}

setInterval(getTracksStatus, 2000)

</script>

<template>
    <div class="mb-3 xl:w-96">
        <div class="input-group relative flex flex-wrap items-stretch w-full mb-4">
            <input type="search" v-model="search"
                class="form-control relative flex-auto min-w-0 block w-full px-3 py-1.5 text-base font-normal text-gray-700 bg-white bg-clip-padding border border-solid border-gray-300 rounded transition ease-in-out m-0 focus:text-gray-700 focus:bg-white focus:border-blue-600 focus:outline-none"
                placeholder="Search" aria-label="Search" aria-describedby="button-addon2">
            <button @click="searchAlbum"
                class="btn inline-block px-6 py-2.5 bg-blue-600 text-white font-medium text-xs leading-tight uppercase rounded shadow-md hover:bg-blue-700 hover:shadow-lg focus:bg-blue-700  focus:shadow-lg focus:outline-none focus:ring-0 active:bg-blue-800 active:shadow-lg transition duration-150 ease-in-out flex items-center"
                type="button" id="button-addon2">
                <svg aria-hidden="true" focusable="false" data-prefix="fas" data-icon="search" class="w-4" role="img"
                    xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">
                    <path fill="currentColor"
                        d="M505 442.7L405.3 343c-4.5-4.5-10.6-7-17-7H372c27.6-35.3 44-79.7 44-128C416 93.1 322.9 0 208 0S0 93.1 0 208s93.1 208 208 208c48.3 0 92.7-16.4 128-44v16.3c0 6.4 2.5 12.5 7 17l99.7 99.7c9.4 9.4 24.6 9.4 33.9 0l28.3-28.3c9.4-9.4 9.4-24.6.1-34zM208 336c-70.7 0-128-57.2-128-128 0-70.7 57.2-128 128-128 70.7 0 128 57.2 128 128 0 70.7-57.2 128-128 128z">
                    </path>
                </svg>
            </button>
        </div>
    </div>

    <div class="flex flex-col">
        <div class="overflow-x-auto sm:-mx-6 lg:-mx-8">
            <div class="py-4 inline-block min-w-full sm:px-6 lg:px-8">
                <div class="overflow-hidden">
                    <table class="min-w-full text-center">
                        <thead class="border-b bg-gray-50">
                            <tr>
                                <th scope="col" class="text-sm font-medium text-gray-900 px-6 py-4">
                                    #TrackID
                                </th>
                                <th scope="col" class="text-sm font-medium text-gray-900 px-6 py-4">
                                    Track name
                                </th>
                                <th scope="col" class="text-sm font-medium text-gray-900 px-6 py-4">
                                    Download status
                                </th>
                            </tr>
                        </thead>
                        <tbody>
                            <tr v-for="track in tracks" :key="track.id" class="bg-white border-b">
                                <td class="px-6 py-4 whitespace-nowrap text-sm font-medium text-gray-900">
                                    {{ track.id }}</td>
                                <td class="text-sm text-gray-900 font-light px-6 py-4 whitespace-nowrap">
                                    <a class="underline text-blue-600 hover:text-blue-800 visited:text-purple-600"
                                        v-if="track.url" v-bind:href="track.url">{{ track.name }}</a>
                                    <span v-else>{{ track.name }}</span>
                                </td>
                                <td class="text-sm text-gray-900 font-light px-6 py-4 whitespace-nowrap">
                                    <div class="flex justify-center items-center">
                                        <div v-if="downloading.status && !track.url"
                                            class="spinner-border animate-spin inline-block w-8 h-8 border-4 rounded-full"
                                            role="status">
                                            <span class="visually-hidden">Loading...</span>
                                        </div>
                                    </div>
                                </td>
                            </tr>
                        </tbody>
                    </table>
                </div>
            </div>
        </div>
    </div>

    <button type="button" @click="downloadTracks"
        class="inline-block px-6 pt-2.5 pb-2 bg-blue-600 text-white font-medium text-xs leading-normal uppercase rounded shadow-md hover:bg-blue-700 hover:shadow-lg focus:bg-blue-700 focus:shadow-lg focus:outline-none focus:ring-0 active:bg-blue-800 active:shadow-lg transition duration-150 ease-in-out flex align-center">
        <svg aria-hidden="true" focusable="false" data-prefix="fas" data-icon="download" class="w-3 mr-2" role="img"
            xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">
            <path fill="currentColor"
                d="M216 0h80c13.3 0 24 10.7 24 24v168h87.7c17.8 0 26.7 21.5 14.1 34.1L269.7 378.3c-7.5 7.5-19.8 7.5-27.3 0L90.1 226.1c-12.6-12.6-3.7-34.1 14.1-34.1H192V24c0-13.3 10.7-24 24-24zm296 376v112c0 13.3-10.7 24-24 24H24c-13.3 0-24-10.7-24-24V376c0-13.3 10.7-24 24-24h146.7l49 49c20.1 20.1 52.5 20.1 72.6 0l49-49H488c13.3 0 24 10.7 24 24zm-124 88c0-11-9-20-20-20s-20 9-20 20 9 20 20 20 20-9 20-20zm64 0c0-11-9-20-20-20s-20 9-20 20 9 20 20 20 20-9 20-20z">
            </path>
        </svg>
        Download
    </button>
</template>