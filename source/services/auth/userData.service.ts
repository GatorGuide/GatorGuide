import { getStorage, ref, listAll, deleteObject } from "firebase/storage";
import { buildFirebaseUserStoragePath } from "@/constants/schema";
import { firebaseApp } from "@/services/firebase/firebase";
import { isStubMode } from "@/services/app/config";

/** Delete a user's uploaded documents from Firebase Storage when their account is removed. */
export async function deleteAllUserStorageFiles(uid: string): Promise<void> {
  if (isStubMode() || !firebaseApp) return;

  try {
    const storage = getStorage(firebaseApp);
    const userFolderRef = ref(storage, buildFirebaseUserStoragePath(uid));
    const listResult = await listAll(userFolderRef);
    await Promise.all(listResult.items.map((itemRef) => deleteObject(itemRef).catch(() => {})));

    for (const prefixRef of listResult.prefixes) {
      const nested = await listAll(prefixRef);
      await Promise.all(nested.items.map((itemRef) => deleteObject(itemRef).catch(() => {})));
    }
  } catch {
    // Storage may be unconfigured or the user's folder may not exist.
  }
}
