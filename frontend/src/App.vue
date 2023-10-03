<script setup>
import TracksTable from './components/TracksTable.vue';
</script>

<template>
  <header>
  </header>

  <main>
    <div class="container my-24 px-6 mx-auto">

      <!-- Section: Design Block -->
      <section class="mb-32 text-gray-800 text-center md:text-left">
        <div class="block rounded-lg shadow-lg bg-white">
          <div class="flex flex-wrap items-center">
            <div class="grow-0 shrink-0 basis-auto block lg:flex w-full lg:w-6/12 xl:w-4/12">
              <img src="https://mdbootstrap.com/img/new/ecommerce/vertical/126.jpg" alt="Trendy Pants and Shoes"
                class="w-full rounded-t-lg lg:rounded-tr-none lg:rounded-bl-lg" />
            </div>
            <div class="px-6 py-12 md:px-12">
              <div class="flex space-x-2 pb-8">
                <h2 class="text-4xl font-medium leading-tight text-gray-800">
                  Spotify
                  <span
                    class="inline-block py-1.5 px-2.5 leading-none text-center whitespace-nowrap align-baseline font-bold bg-blue-600 text-white rounded">RIPPER</span>
                </h2>
              </div>
              <TracksTable />
            </div>
          </div>
        </div>
      </section>
    </div>

  </main>
</template>

