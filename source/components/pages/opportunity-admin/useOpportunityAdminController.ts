import { useCallback, useEffect, useMemo, useState } from "react";
import { Alert } from "react-native";

import { STARTER_OPPORTUNITIES } from "@/constants/starter-opportunities";
import { OPPORTUNITY_STATUSES, normalizeOpportunity, type Opportunity } from "@/constants/opportunities";
import { useAppData } from "@/hooks/use-app-data";
import { useOpportunities } from "@/hooks/use-opportunities";
import {
  OpportunityGatewayError,
  opportunityGatewayService,
  type OpportunityAdminAccessResponse,
} from "@/services/opportunities/opportunity-gateway.service";
import { buildScholarshipExportFile } from "@/services/opportunities/scholarship-export.service";
import { saveTextFileForUser } from "@/services/storage/file-system-adapter.service";
import {
  buildDraftFromOpportunity,
  buildSaveInput,
  getPublishableOpportunityCatalog,
  createBlankDraft,
  validateOpportunityAdminDraft,
  type OpportunityAdminDraft,
} from "@/components/pages/opportunity-admin/opportunity-admin-draft";

export function useOpportunityAdminController() {
  const { state } = useAppData();
  const { opportunities, refreshOpportunities, isRefreshing } = useOpportunities();
  const [query, setQuery] = useState("");
  const [selectedOpportunityId, setSelectedOpportunityId] = useState<string | null>(null);
  const [draft, setDraft] = useState<OpportunityAdminDraft>(() => createBlankDraft());
  const [catalogDraft, setCatalogDraft] = useState<Opportunity[] | null>(null);
  const [access, setAccess] = useState<OpportunityAdminAccessResponse | null>(null);
  const [isCheckingAccess, setIsCheckingAccess] = useState(false);
  const [accessError, setAccessError] = useState("");
  const [isSaving, setIsSaving] = useState(false);
  const [saveMessage, setSaveMessage] = useState("");
  const [isExportingCatalog, setIsExportingCatalog] = useState(false);
  const [catalogExportMessage, setCatalogExportMessage] = useState("");
  const [isExportingScholarships, setIsExportingScholarships] = useState(false);
  const [exportMessage, setExportMessage] = useState("");

  const currentCatalog = catalogDraft ?? opportunities;
  const starterIds = useMemo(
    () => new Set(STARTER_OPPORTUNITIES.map((item) => item.opportunityId)),
    []
  );
  const signedInUser = state.user && !state.user.isGuest ? state.user : null;
  const scholarshipExportPreview = useMemo(
    () => buildScholarshipExportFile(currentCatalog),
    [currentCatalog]
  );

  const selectedOpportunity = useMemo(
    () =>
      selectedOpportunityId
        ? currentCatalog.find((item) => item.opportunityId === selectedOpportunityId) ?? null
        : null,
    [currentCatalog, selectedOpportunityId]
  );

  const filteredOpportunities = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    if (!normalizedQuery) return currentCatalog;
    return currentCatalog.filter((opportunity) =>
      [
        opportunity.title,
        opportunity.organizationName,
        opportunity.summary,
        opportunity.opportunityId,
        opportunity.type,
        opportunity.status,
      ]
        .join(" ")
        .toLowerCase()
        .includes(normalizedQuery)
    );
  }, [currentCatalog, query]);

  const canDeleteSelected =
    !!selectedOpportunity &&
    selectedOpportunity.source.kind === "manual" &&
    !starterIds.has(selectedOpportunity.opportunityId);

  const updateCatalog = useCallback((mutate: (items: Opportunity[]) => Opportunity[]) => {
    setCatalogDraft((current) => mutate(current ?? opportunities));
  }, [opportunities]);

  const loadAccess = useCallback(async () => {
    if (!signedInUser) {
      setAccess(null);
      setAccessError("");
      setIsCheckingAccess(false);
      return;
    }

    setIsCheckingAccess(true);
    setAccessError("");
    try {
      setAccess(await opportunityGatewayService.getOpportunityAdminAccess());
    } catch (error) {
      setAccess(null);
      setAccessError(
        error instanceof OpportunityGatewayError ? error.message : "Could not verify admin access."
      );
    } finally {
      setIsCheckingAccess(false);
    }
  }, [signedInUser]);

  useEffect(() => {
    void loadAccess();
  }, [loadAccess]);

  const handleSelectOpportunity = useCallback((opportunity: Opportunity) => {
    setSelectedOpportunityId(opportunity.opportunityId);
    setDraft(buildDraftFromOpportunity(opportunity));
    setSaveMessage("");
  }, []);

  const handleCreateNew = useCallback(() => {
    setSelectedOpportunityId(null);
    setDraft(createBlankDraft());
    setSaveMessage("");
  }, []);

  const handleSave = useCallback(async () => {
    const validation = validateOpportunityAdminDraft(draft);
    if (!validation.isValid) {
      Alert.alert(validation.title, validation.message);
      return;
    }

    setIsSaving(true);
    setSaveMessage("");
    try {
      const response = await opportunityGatewayService.normalizeManualOpportunity(
        buildSaveInput(draft, selectedOpportunity)
      );
      const normalized = normalizeOpportunity(response.opportunity);
      if (!normalized.opportunityId) throw new Error("Opportunity response did not include a valid ID.");

      updateCatalog((items) => [
        ...items.filter((item) => item.opportunityId !== normalized.opportunityId),
        normalized,
      ]);
      setSelectedOpportunityId(normalized.opportunityId);
      setDraft(buildDraftFromOpportunity(normalized));
      setSaveMessage("Saved to a local catalog draft. Export the GitHub JSON and commit it to publish changes.");
    } catch (error) {
      setSaveMessage(
        error instanceof OpportunityGatewayError
          ? error.message
          : error instanceof Error ? error.message : "Opportunity draft save failed."
      );
    } finally {
      setIsSaving(false);
    }
  }, [draft, selectedOpportunity, updateCatalog]);

  const handleArchive = useCallback((archived: boolean) => {
    if (!selectedOpportunityId) return;
    const now = new Date().toISOString();
    const status = archived ? OPPORTUNITY_STATUSES.archived : OPPORTUNITY_STATUSES.active;
    updateCatalog((items) => items.map((item) => item.opportunityId === selectedOpportunityId
      ? { ...item, status, updatedAt: now, source: { ...item.source, verifiedAt: now } }
      : item
    ));
    setDraft((current) => ({ ...current, status }));
    setSaveMessage("Catalog draft updated locally. Export the GitHub JSON and commit it to publish changes.");
  }, [selectedOpportunityId, updateCatalog]);

  const handleDelete = useCallback(() => {
    if (!selectedOpportunityId || !canDeleteSelected) return;
    updateCatalog((items) => items.filter((item) => item.opportunityId !== selectedOpportunityId));
    setSelectedOpportunityId(null);
    setDraft(createBlankDraft());
    setSaveMessage("Manual opportunity removed from the local draft. Export the GitHub JSON to publish the deletion.");
  }, [canDeleteSelected, selectedOpportunityId, updateCatalog]);

  const handleExportCatalog = useCallback(async () => {
    setIsExportingCatalog(true);
    setCatalogExportMessage("");
    try {
      const fileName = "starter-opportunities.json";
      const publishableCatalog = getPublishableOpportunityCatalog(currentCatalog);
      const content = `${JSON.stringify(publishableCatalog, null, 2)}\n`;
      const savedFile = await saveTextFileForUser({
        fileName,
        content,
        mimeType: "application/json;charset=utf-8",
      });
      if (savedFile.delivery === "filesystem") {
        Alert.alert("Catalog export ready", `${fileName} was saved to app documents.`);
      }
      setCatalogExportMessage(`Exported ${publishableCatalog.length} reviewed opportunities. AI-generated deadlines are excluded until reviewed and saved as manual records. Replace source/data/${fileName} and commit to publish.`);
    } catch (error) {
      const message = error instanceof Error ? error.message : "Could not export the catalog JSON.";
      setCatalogExportMessage(message);
      Alert.alert("Catalog export failed", message);
    } finally {
      setIsExportingCatalog(false);
    }
  }, [currentCatalog]);

  const handleExportScholarships = useCallback(async () => {
    if (!scholarshipExportPreview.scholarshipCount) {
      Alert.alert("No scholarships found", "There are no scholarship records to export yet.");
      return;
    }

    setIsExportingScholarships(true);
    setExportMessage("");
    try {
      const savedFile = await saveTextFileForUser({
        fileName: scholarshipExportPreview.fileName,
        content: scholarshipExportPreview.content,
        mimeType: "text/tab-separated-values;charset=utf-8",
      });
      if (savedFile.delivery === "filesystem") {
        Alert.alert("Scholarship export ready", `${scholarshipExportPreview.fileName} was saved to app documents.`);
      }
      setExportMessage(
        `Exported ${scholarshipExportPreview.scholarshipCount} scholarships (${scholarshipExportPreview.legacyCount} legacy).`
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : "Could not create the scholarship export.";
      setExportMessage(message);
      Alert.alert("Scholarship export failed", message);
    } finally {
      setIsExportingScholarships(false);
    }
  }, [scholarshipExportPreview]);

  const updateDraft = useCallback(
    <K extends keyof OpportunityAdminDraft>(key: K, value: OpportunityAdminDraft[K]) => {
      setDraft((current) => ({ ...current, [key]: value }));
    },
    []
  );

  return {
    access,
    accessError,
    canDeleteSelected,
    catalogExportMessage,
    draft,
    exportMessage,
    filteredOpportunities,
    handleArchive,
    handleCreateNew,
    handleDelete,
    handleExportCatalog,
    handleExportScholarships,
    handleSave,
    handleSelectOpportunity,
    isCheckingAccess,
    isExportingCatalog,
    isExportingScholarships,
    isRefreshing,
    isSaving,
    loadAccess,
    opportunities: currentCatalog,
    query,
    refreshOpportunities,
    saveMessage,
    scholarshipExportPreview,
    selectedOpportunity,
    selectedOpportunityId,
    setQuery,
    signedInUser,
    updateDraft,
  };
}
