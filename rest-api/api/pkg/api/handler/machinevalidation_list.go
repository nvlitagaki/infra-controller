// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package handler

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strconv"
	"time"

	"github.com/labstack/echo/v4"

	"github.com/NVIDIA/infra-controller/rest-api/api/pkg/api/handler/util/common"
	"github.com/NVIDIA/infra-controller/rest-api/api/pkg/api/model"
	sc "github.com/NVIDIA/infra-controller/rest-api/api/pkg/client/site"
	cutil "github.com/NVIDIA/infra-controller/rest-api/common/pkg/util"
	cdb "github.com/NVIDIA/infra-controller/rest-api/db/pkg/db"
	corev1 "github.com/NVIDIA/infra-controller/rest-api/proto/core/gen/v1"
)

// ListMachineValidationRunsHandler lists validation runs across one site.
type ListMachineValidationRunsHandler struct {
	dbSession  *cdb.Session
	scp        *sc.ClientPool
	tracerSpan *cutil.TracerSpan
}

// NewListMachineValidationRunsHandler returns a site-wide validation run list handler.
func NewListMachineValidationRunsHandler(dbSession *cdb.Session, scp *sc.ClientPool) ListMachineValidationRunsHandler {
	return ListMachineValidationRunsHandler{
		dbSession:  dbSession,
		scp:        scp,
		tracerSpan: cutil.NewTracerSpan(),
	}
}

// Handle godoc
// @Summary List Machine validation runs
// @Description List Machine validation runs across a Site
// @Tags Machine Validation
// @Produce json
// @Security ApiKeyAuth
// @Param org path string true "Name of NGC organization"
// @Param siteId query string true "ID of Site"
// @Param machineId query string false "ID of Machine"
// @Param startedAfter query string false "Inclusive run start-time lower bound (RFC3339)"
// @Param startedBefore query string false "Exclusive run start-time upper bound (RFC3339)"
// @Param pageSize query integer false "Number of runs to return (default 20, maximum 100)"
// @Param pageToken query string false "Opaque token returned by the previous page"
// @Success 200 {object} []model.APIMachineValidationRun
// @Router /v2/org/{org}/nico/machine/validation/run [get]
func (handler ListMachineValidationRunsHandler) Handle(c echo.Context) error {
	org, dbUser, ctx, logger, handlerSpan := common.SetupHandler("MachineValidationRun", "List", c, handler.tracerSpan)
	if handlerSpan != nil {
		defer handlerSpan.End()
	}

	request, err := parseMachineValidationRunListRequest(c)
	if err != nil {
		return cutil.NewAPIErrorResponse(c, http.StatusBadRequest, err.Error(), nil)
	}

	stc, siteID, apiErr := common.AuthorizeProviderSiteForCore(common.AuthorizeProviderSiteForCoreInput{
		Ctx:       ctx,
		Logger:    logger,
		DBSession: handler.dbSession,
		SCP:       handler.scp,
		Org:       org,
		User:      dbUser,
		SiteID:    request.SiteID,
	})
	if apiErr != nil {
		return cutil.NewAPIErrorResponse(c, apiErr.Code, apiErr.Message, apiErr.Data)
	}

	coreResponse := &corev1.ListMachineValidationRunsResponse{}
	apiErr = common.ExecuteCoreGRPC(
		ctx,
		stc,
		corev1.Forge_ListMachineValidationRuns_FullMethodName,
		request.ToProto(),
		coreResponse,
		"",
	)
	if apiErr != nil {
		logAPIError(logger, apiErr, "failed to list Machine validation runs")
		return cutil.NewAPIErrorResponse(c, apiErr.Code, apiErr.Message, nil)
	}

	response := make([]*model.APIMachineValidationRun, 0, len(coreResponse.GetRuns()))
	for _, run := range coreResponse.GetRuns() {
		response = append(response, model.NewAPIMachineValidationRun(run))
	}
	page := model.APIMachineValidationRunPage{
		PageSize:      request.EffectivePageSize(),
		Total:         coreResponse.GetTotalSize(),
		NextPageToken: coreResponse.GetNextPageToken(),
	}
	pageHeader, err := json.Marshal(page)
	if err != nil {
		logger.Error().Err(err).Msg("failed to serialize Machine validation run pagination header")
		return cutil.NewAPIErrorResponse(c, http.StatusInternalServerError, "Failed to serialize pagination response", nil)
	}
	c.Response().Header().Set("X-Pagination", string(pageHeader))
	logger.Info().Str("siteID", siteID).Uint64("total", coreResponse.GetTotalSize()).Msg("listed Machine validation runs")
	return c.JSON(http.StatusOK, response)
}

func parseMachineValidationRunListRequest(c echo.Context) (model.APIMachineValidationRunListRequest, error) {
	request := model.APIMachineValidationRunListRequest{
		SiteID:    c.QueryParam("siteId"),
		MachineID: c.QueryParam("machineId"),
		State:     model.APIMachineValidationState(c.QueryParam("state")),
		PageToken: c.QueryParam("pageToken"),
	}

	pageSize := c.QueryParam("pageSize")
	if pageSize != "" {
		parsedPageSize, err := strconv.ParseUint(pageSize, 10, 32)
		if err != nil || parsedPageSize == 0 {
			return request, fmt.Errorf("pageSize must be an integer greater than 0")
		}
		request.PageSize = uint32(parsedPageSize)
	}

	startedAfter := c.QueryParam("startedAfter")
	if startedAfter != "" {
		parsedStartedAfter, err := time.Parse(time.RFC3339Nano, startedAfter)
		if err != nil {
			return request, fmt.Errorf("startedAfter must be an RFC3339 timestamp")
		}
		request.StartedAfter = &parsedStartedAfter
	}

	startedBefore := c.QueryParam("startedBefore")
	if startedBefore != "" {
		parsedStartedBefore, err := time.Parse(time.RFC3339Nano, startedBefore)
		if err != nil {
			return request, fmt.Errorf("startedBefore must be an RFC3339 timestamp")
		}
		request.StartedBefore = &parsedStartedBefore
	}

	err := request.Validate()
	return request, err
}
