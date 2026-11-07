package dev.ferrovoz.api.internal;

import dev.ferrovoz.api.FerrovozApi;

import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Where Ferrovoz puts its implementation. Internal: plugins use
 * {@link FerrovozApi#get()} and friends, not this class.
 */
public final class ApiAccess {
    private static final Logger LOG = Logger.getLogger("ferrovoz");
    private static FerrovozApi current;
    private static final List<Consumer<FerrovozApi>> waiting = new ArrayList<>();

    private ApiAccess() {}

    public static synchronized FerrovozApi current() {
        return current;
    }

    public static FerrovozApi require() {
        FerrovozApi api = current();
        if (api == null) throw new IllegalStateException("Ferrovoz is not loaded (or has no native library for this machine)");
        return api;
    }

    public static void whenReady(Consumer<FerrovozApi> action) {
        FerrovozApi api;
        synchronized (ApiAccess.class) {
            api = current;
            if (api == null) {
                waiting.add(action);
                return;
            }
        }
        action.accept(api);
    }

    /** Called by Ferrovoz when it loads. Runs whatever was waiting. */
    public static void register(FerrovozApi api) {
        List<Consumer<FerrovozApi>> run;
        synchronized (ApiAccess.class) {
            current = api;
            run = new ArrayList<>(waiting);
            waiting.clear();
        }
        for (Consumer<FerrovozApi> action : run) {
            try {
                action.accept(api);
            } catch (RuntimeException e) {
                LOG.log(Level.WARNING, "a FerrovozApi.whenReady callback failed", e);
            }
        }
    }

    public static synchronized void unregister(FerrovozApi api) {
        if (current == api) current = null;
    }
}
