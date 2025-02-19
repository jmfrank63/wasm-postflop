/* copied from https://github.com/GoogleChromeLabs/wasm-bindgen-rayon */

/*
 * Copyright 2022 Google Inc. All Rights Reserved.
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *     http://www.apache.org/licenses/LICENSE-2.0
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

// Note: `atomics` is whitelisted in `target_feature` detection, but `bulk-memory` isn't,
// so we can check only presence of the former. This should be enough to catch most common
// mistake (forgetting to pass `RUSTFLAGS` altogether).
#[cfg(not(target_feature = "atomics"))]
compile_error!("Did you forget to enable `atomics` and `bulk-memory` features as outlined in wasm-bindgen-rayon README?");

use js_sys::Promise;
use spmc::{channel, Receiver, Sender};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsValue;
use std::sync::Mutex;
use once_cell::sync::Lazy;

// Naming is a workaround for https://github.com/rustwasm/wasm-bindgen/issues/2429
// and https://github.com/rustwasm/wasm-bindgen/issues/1762.
#[allow(non_camel_case_types)]
#[wasm_bindgen]
#[doc(hidden)]
pub struct wbg_rayon_PoolBuilder {
    num_threads: usize,
    sender: Sender<rayon::ThreadBuilder>,
    receiver: Receiver<rayon::ThreadBuilder>,
}

#[wasm_bindgen(module = "/workerHelpers.js")]
extern "C" {
    #[wasm_bindgen(js_name = startWorkers)]
    fn start_workers(module: JsValue, memory: JsValue, builder: wbg_rayon_PoolBuilder) -> Promise;
    #[wasm_bindgen(js_name = terminateWorkers)]
    fn terminate_workers() -> Promise;
}

// pub static mut THREAD_POOL: Option<rayon::ThreadPool> = None;
pub static THREAD_POOL: Lazy<Mutex<Option<rayon::ThreadPool>>> = Lazy::new(|| Mutex::new(None));

// pub fn is_thread_pool_none() -> bool {
//     THREAD_POOL.lock().unwrap().is_none()
// }

#[wasm_bindgen]
impl wbg_rayon_PoolBuilder {
    fn new(num_threads: usize) -> Self {
        let (sender, receiver) = channel();
        Self {
            num_threads,
            sender,
            receiver,
        }
    }

    #[wasm_bindgen(js_name = numThreads)]
    pub fn num_threads(&self) -> usize {
        self.num_threads
    }

    pub fn receiver(&self) -> *const Receiver<rayon::ThreadBuilder> {
        &self.receiver
    }

    // This should be called by the JS side once all the Workers are spawned.
    // Important: it must take `self` by reference, otherwise
    // `start_worker_thread` will try to receive a message on a moved value.
        pub fn build(&mut self) {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.num_threads)
            .spawn_handler(move |thread| {
                self.sender.send(thread).unwrap_throw();
                Ok(())
            })
            .build()
            .unwrap_throw();

        // Lock and update the inner Option of our THREAD_POOL.
        THREAD_POOL.lock().unwrap_throw().replace(pool);
    }
}

#[wasm_bindgen(js_name = initThreadPool)]
#[doc(hidden)]
pub fn init_thread_pool(num_threads: usize) -> Promise {
    // Lock the Mutex and check if a thread pool already exists.
    if !THREAD_POOL.lock().unwrap_throw().is_none() {
        panic!("Thread pool is already initialized");
    }
    start_workers(
        wasm_bindgen::module(),
        wasm_bindgen::memory(),
        wbg_rayon_PoolBuilder::new(num_threads),
    )
}

#[wasm_bindgen(js_name = exitThreadPool)]
#[doc(hidden)]
pub fn exit_thread_pool() -> Promise {
    if THREAD_POOL.lock().unwrap_throw().is_none() {
        panic!("Thread pool is not initialized");
    }
    let promise = terminate_workers();
    // Remove the thread pool by taking it out of the Option.
    THREAD_POOL.lock().unwrap_throw().take();
    promise
}

#[wasm_bindgen]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[doc(hidden)]
pub fn wbg_rayon_start_worker(receiver: *const Receiver<rayon::ThreadBuilder>)
where
    // Statically assert that it's safe to accept `Receiver` from another thread.
    Receiver<rayon::ThreadBuilder>: Sync,
{
    // This is safe, because we know it came from a reference to PoolBuilder,
    // allocated on the heap by wasm-bindgen and dropped only once all the
    // threads are running.
    //
    // The only way to violate safety is if someone externally calls
    // `exports.wbg_rayon_start_worker(garbageValue)`, but then no Rust tools
    // would prevent us from issues anyway.
    let receiver = unsafe { &*receiver };
    // Wait for a task (`ThreadBuilder`) on the channel, and, once received,
    // start executing it.
    //
    // On practice this will start running Rayon's internal event loop.
    receiver.recv().unwrap_throw().run()
}
