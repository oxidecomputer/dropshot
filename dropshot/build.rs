// Copyright 2023 Oxide Computer Company
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    if !version_check::is_min_version("1.59").unwrap_or(false) {
        println!("cargo:rustc-cfg=usdt_need_asm");
    }

    #[cfg(target_os = "macos")]
    if version_check::supports_feature("asm_sym").unwrap_or(false)
        && !version_check::is_min_version("1.67").unwrap_or(false)
    {
        println!("cargo:rustc-cfg=usdt_need_asm_sym");
    }
}
