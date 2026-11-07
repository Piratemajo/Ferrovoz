package dev.ferrovoz.api.internal;

import dev.ferrovoz.api.FerrovozApi;

import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Dónde Ferrovoz deja su implementación. Interno: los plugins usan
 * {@link FerrovozApi#get()} y compañía, no esta clase.
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
        if (api == null) throw new IllegalStateException("Ferrovoz no está cargado (o no tiene librería nativa para esta máquina)");
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

    /** Lo llama Ferrovoz al cargar. Ejecuta lo que estaba esperando. */
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
