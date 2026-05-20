use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    view! {
        <main class="shell">
            <section class="panel hero">
                <p class="eyebrow">"TypeKart Web"</p>
                <h1>"Browser racing shell"</h1>
                <p class="lede">
                    "This is the first browser surface for the future online client. Gameplay is not wired yet."
                </p>
                <div class="actions" aria-label="Game setup placeholders">
                    <button type="button">"Create room"</button>
                    <button type="button" class="secondary">"Join room"</button>
                </div>
            </section>

            <section class="panel track-preview" aria-label="Static race preview">
                <div class="track-text">
                    <span>"spark"</span>
                    <span>"river"</span>
                    <span>"focus"</span>
                    <span>"cyclone"</span>
                    <span>"finish"</span>
                </div>
                <div class="lane self">
                    <span class="marker">"███"</span>
                    <span>"you"</span>
                </div>
                <div class="lane">
                    <span class="marker red">"███"</span>
                    <span>"rival"</span>
                </div>
                <div class="map" aria-hidden="true">"|***-----------------------------|"</div>
            </section>
        </main>
    }
}
